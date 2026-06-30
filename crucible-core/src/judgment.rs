//! The adjudication queue: an ordered VIEW over a [`GradeResult`] that turns the
//! ambiguous half of a grade into a judge's work list, plus the append-only
//! [`Label`] each decision produces (backlog 002 child 4 — the wedge keystone).
//!
//! The deterministic floor ([`mod@crate::grade`]) auto-resolves only confident
//! agreement: a candidate that matches a key row is `matched` and needs no human.
//! What is left is exactly what a judge must rule on — the `disputed` candidates
//! the review raised that no key row confirms — enriched with the *recoverable*
//! misses (key rows a disputed candidate agrees with on location but not
//! category, which [`recoverable_misses`] counts). A
//! [`JudgmentQueue`] is that work list, with three contracts:
//!
//! - It is a **view, not a store** (backlog 004): [`build_queue`] recomputes it
//!   from a [`GradeResult`] every time, persisting no third copy of the findings.
//!   The queue *is* the artifact — it carries a `schema_version` and round-trips
//!   through serde, so the phone adjudication UI (005) and the export (002.5)
//!   read one shared shape.
//! - Its items are **ordered by decision value**: a disputed candidate that could
//!   recover a miss comes before a plain dispute, because the deterministic floor
//!   most likely erred on the former — it double-penalized a real finding as both
//!   a false positive (`disputed`) and a false negative (`missed`). Within each
//!   group input order is preserved, so the order is a deterministic function of
//!   the grade.
//! - Only `disputed` candidates become items, each carrying the source
//!   [`Finding`](crate::Finding) id a [`Label`] references. A `missed` key row the
//!   review never surfaced is the *absence* of a finding — there is nothing to
//!   label — so it appears only as `recoverable_against` context on the disputed
//!   item it co-locates with, never as a standalone item. `matched` candidates
//!   are already resolved; they are summarized in [`GradeSummary`], not queued.
//!
//! [`apply_label`] is the single minting path from a queue item to a [`Label`]:
//! it stamps the item's finding id onto a judge's [`Verdict`] + [`Disposition`]
//! and the [`LabelConditions`] (latency, grader-blindness, timestamp) the
//! calibration story (005) needs. The phone UI and the headless `crucible
//! adjudicate --apply` loop both go through it, so every label is built the same
//! way and the append-only history stays consistent.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::grade::{location_agrees, recoverable_misses, GradeResult};
use crate::key::KeyFinding;
use crate::label::{Label, LABEL_SCHEMA};
use crate::{Disposition, Verdict};

/// Schema identifier for a persisted [`JudgmentQueue`].
pub const JUDGMENT_QUEUE_SCHEMA: &str = "crucible.judgment_queue.v1";

/// The grade partition a [`JudgmentQueue`] views, as plain counts.
///
/// Carried so a consumer sees the whole matched / disputed / missed picture — and
/// how many of the misses are recoverable — without re-running
/// [`grade`](crate::grade()). Derived, never authoritative: the source of truth is
/// the [`GradeResult`] the queue was built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradeSummary {
    /// Candidate rows that agreed with a key row.
    pub matched: usize,
    /// Candidate rows with no matching key row — the queue's items come from here.
    pub disputed: usize,
    /// Key rows no candidate matched.
    pub missed: usize,
    /// Of `missed`, how many a disputed candidate could still recover
    /// ([`recoverable_misses`]).
    pub recoverable_misses: usize,
}

impl GradeSummary {
    /// Count the four buckets of a grade.
    pub fn from_grade(grade: &GradeResult) -> Self {
        GradeSummary {
            matched: grade.matched.len(),
            disputed: grade.disputed.len(),
            missed: grade.missed.len(),
            recoverable_misses: recoverable_misses(grade),
        }
    }
}

/// One finding a judge must rule on: a `disputed` candidate plus the context that
/// decides it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgmentItem {
    /// The id a [`Label`] for this item references — the candidate's source
    /// [`Finding`](crate::Finding) id when present, else a positional `item-{n}`
    /// fallback so an id-less candidate still addresses a unique label target.
    pub finding_id: String,
    /// The disputed candidate row, projected for the judge to read
    /// (file / line / category / severity / description).
    pub candidate: KeyFinding,
    /// Missed key rows this candidate agrees with on location but not category —
    /// the recoverable misses a `keep` verdict would resolve. Empty for a plain
    /// dispute (no key row at this location); non-empty marks a recoverable item.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recoverable_against: Vec<KeyFinding>,
}

impl JudgmentItem {
    /// Whether ruling this finding `keep` would recover a miss: it shares a
    /// location with at least one `missed` key row the category-strict matcher
    /// dropped. The queue orders these first.
    pub fn is_recoverable(&self) -> bool {
        !self.recoverable_against.is_empty()
    }
}

/// An ordered adjudication queue: the schema-stamped artifact the phone UI (005)
/// and export (002.5) consume.
///
/// A view over a [`GradeResult`] (see [`build_queue`]): `items` are the disputed
/// candidates to rule on (recoverable first), `summary` is the grade partition
/// they came from, and `labels` accumulates the append-only [`Label`]s applied so
/// far (empty for a freshly built, unlabeled queue).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgmentQueue {
    /// Schema identifier; defaults to [`JUDGMENT_QUEUE_SCHEMA`]. A present value
    /// is validated on load — an unknown schema is rejected, not assumed v1.
    #[serde(
        default = "judgment_queue_schema",
        deserialize_with = "deserialize_queue_schema"
    )]
    pub schema_version: String,
    /// The grade partition this queue views.
    pub summary: GradeSummary,
    /// The ordered items needing adjudication.
    pub items: Vec<JudgmentItem>,
    /// Append-only labels applied to this queue's items. Empty (and omitted on
    /// the wire) until a judge or the `adjudicate --apply` loop commits decisions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Label>,
}

impl JudgmentQueue {
    /// The item a label addresses, by finding id, if it is in the queue.
    ///
    /// The validation hook the `--apply` loop uses: a label whose `finding_id`
    /// names no queue item is a decision about a finding that needs no
    /// adjudication, and must be rejected rather than silently dropped.
    ///
    /// Finding ids are **unique within a queue** [`build_queue`] builds: two
    /// disputed findings that carry the same source id are disambiguated
    /// (`unique_item_id`), so this first match is also the *only* match — a
    /// label never silently binds to the wrong one of a colliding pair.
    pub fn item(&self, finding_id: &str) -> Option<&JudgmentItem> {
        self.items.iter().find(|i| i.finding_id == finding_id)
    }
}

fn judgment_queue_schema() -> String {
    JUDGMENT_QUEUE_SCHEMA.to_string()
}

fn deserialize_queue_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, JUDGMENT_QUEUE_SCHEMA)
}

/// Build the adjudication queue from a grade: order the disputed candidates by
/// decision value and attach the misses each could recover.
///
/// Recoverable disputes (a candidate co-located with a `missed` key row) come
/// first, then plain disputes; input order is preserved within each group, so the
/// queue is a deterministic function of the grade. `matched` and non-recoverable
/// `missed` rows are summarized in [`GradeSummary`] but never become items — a
/// matched candidate needs no judgment, and a miss the review never surfaced has
/// no finding to label.
pub fn build_queue(grade: &GradeResult) -> JudgmentQueue {
    // Partition disputed candidates into recoverable (co-located with at least
    // one miss) and plain, preserving input order within each group.
    let mut recoverable: Vec<(&KeyFinding, Vec<KeyFinding>)> = Vec::new();
    let mut plain: Vec<&KeyFinding> = Vec::new();
    for cand in &grade.disputed {
        let against: Vec<KeyFinding> = grade
            .missed
            .iter()
            .filter(|m| location_agrees(cand, m))
            .cloned()
            .collect();
        if against.is_empty() {
            plain.push(cand);
        } else {
            recoverable.push((cand, against));
        }
    }

    let mut items = Vec::with_capacity(recoverable.len() + plain.len());
    let mut used = HashSet::new();
    for (cand, against) in recoverable {
        let finding_id = unique_item_id(cand, items.len(), &mut used);
        items.push(JudgmentItem {
            finding_id,
            candidate: cand.clone(),
            recoverable_against: against,
        });
    }
    for cand in plain {
        let finding_id = unique_item_id(cand, items.len(), &mut used);
        items.push(JudgmentItem {
            finding_id,
            candidate: cand.clone(),
            recoverable_against: Vec::new(),
        });
    }

    JudgmentQueue {
        schema_version: judgment_queue_schema(),
        summary: GradeSummary::from_grade(grade),
        items,
        labels: Vec::new(),
    }
}

/// A queue-unique label target for a disputed candidate: its source id, or a
/// positional `item-{position}` fallback for an id-less row — disambiguated when
/// that id is already taken.
///
/// Two distinct findings can carry the *same* source id (an upstream review may
/// repeat `F1`). A bare id then aliases two findings, and [`JudgmentQueue::item`]
/// would bind a label to whichever it found first — the wrong finding half the
/// time. So the first use of an id keeps it bare and every repeat is suffixed
/// with the item's queue `position` (`F1#3`), which is unique per item; `used`
/// carries the ids already assigned. The trailing loop is a belt-and-suspenders
/// guard for the rare case where a suffixed form equals an unrelated real id.
fn unique_item_id(candidate: &KeyFinding, position: usize, used: &mut HashSet<String>) -> String {
    let base = candidate
        .source_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("item-{position}"));
    let mut id = if used.contains(&base) {
        format!("{base}#{position}")
    } else {
        base
    };
    while !used.insert(id.clone()) {
        id = format!("{id}#{position}");
    }
    id
}

/// Collapse an append-only label history to the one effective label per finding.
///
/// Labels are append-only: a correction is a *new* [`Label`] with a later
/// `timestamp`, never a mutation of an existing one (see the [`label`] module
/// docs). One finding can therefore carry several labels — an initial ruling and
/// later corrections — of which only the latest is in force. This returns the
/// winning label per `finding_id`: the one with the latest `timestamp`, ties
/// broken by position so the last-appended correction wins (equal or empty
/// timestamps included). Findings keep first-seen order, so a reconciled history
/// renders in the order it was committed.
///
/// Both the export ([`adjudications_from_queue`](crate::adjudications_from_queue))
/// and the `adjudicate --apply` mint path run every label set through this, so a
/// retracted ACCEPT never double-extends the key or bumps the version twice, and
/// a duplicated decision never double-counts.
///
/// [`label`]: crate::label
pub fn reconcile_labels(labels: &[Label]) -> Vec<&Label> {
    let mut order: Vec<&str> = Vec::new();
    let mut winner: HashMap<&str, &Label> = HashMap::new();
    for label in labels {
        let fid = label.finding_id.as_str();
        match winner.get_mut(fid) {
            Some(current) => {
                // Latest timestamp wins; an equal (or empty) timestamp falls to
                // this later-appended label — the append-only correction.
                if label.timestamp >= current.timestamp {
                    *current = label;
                }
            }
            None => {
                order.push(fid);
                winner.insert(fid, label);
            }
        }
    }
    order.into_iter().map(|fid| winner[fid]).collect()
}

/// The calibration-validity conditions a [`Label`] is committed under.
///
/// Bundles the three context fields a [`Label`] needs beyond its verdict and
/// scope — how long the judge took, whether they saw the grader's verdict first,
/// and when they committed — so [`apply_label`] takes one decision context, not
/// three loose arguments. Backlog 005 reads these to decide whether a label is
/// valid calibration data (a slow, grader-revealed judgment is not). Defaults to
/// a blind, unmeasured, untimestamped decision.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelConditions {
    /// Milliseconds from card presentation to commit. Defaults to `0`.
    #[serde(default)]
    pub latency_ms: u64,
    /// Whether the grader's verdict was visible before commit. Defaults to `false`.
    #[serde(default)]
    pub saw_grader_before_commit: bool,
    /// Caller-supplied RFC 3339 commit timestamp. Defaults to empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timestamp: String,
}

/// Apply a judge's verdict and scope to a queue item, minting the append-only
/// [`Label`] that records the decision against the item's finding.
///
/// The one path from an adjudication decision to a label: it stamps the item's
/// `finding_id` (so the label traces back to the source finding), the current
/// [`LABEL_SCHEMA`], and the [`LabelConditions`] the decision was made under. The
/// phone UI (005) and the headless `crucible adjudicate --apply` loop both call
/// it, so every label is built identically. Appending the result to a
/// [`JudgmentQueue`]'s `labels` preserves the append-only history; this function
/// never mutates an existing label.
pub fn apply_label(
    item: &JudgmentItem,
    verdict: Verdict,
    disposition: Disposition,
    conditions: &LabelConditions,
) -> Label {
    Label {
        schema_version: LABEL_SCHEMA.to_string(),
        finding_id: item.finding_id.clone(),
        verdict,
        disposition,
        latency_ms: conditions.latency_ms,
        saw_grader_before_commit: conditions.saw_grader_before_commit,
        timestamp: conditions.timestamp.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grade::Match;

    /// A candidate row (review side) carrying its source finding id.
    fn cand(file: &str, line: u32, category: &str, source_id: &str) -> KeyFinding {
        KeyFinding {
            file: file.to_string(),
            line,
            category: category.to_string(),
            severity: "blocking".to_string(),
            description: "candidate".to_string(),
            source_id: Some(source_id.to_string()),
        }
    }

    /// An answer-key row (key side), no source id.
    fn key(file: &str, line: u32, category: &str) -> KeyFinding {
        KeyFinding {
            file: file.to_string(),
            line,
            category: category.to_string(),
            severity: String::new(),
            description: "key".to_string(),
            source_id: None,
        }
    }

    #[test]
    fn build_queue_of_empty_grade_is_empty() {
        let q = build_queue(&GradeResult {
            matched: Vec::new(),
            disputed: Vec::new(),
            missed: Vec::new(),
        });
        assert!(q.items.is_empty());
        assert!(q.labels.is_empty());
        assert_eq!(
            q.summary,
            GradeSummary {
                matched: 0,
                disputed: 0,
                missed: 0,
                recoverable_misses: 0,
            }
        );
        assert_eq!(q.schema_version, JUDGMENT_QUEUE_SCHEMA);
    }

    #[test]
    fn build_queue_orders_recoverable_disputes_before_plain() {
        // F1 is a plain dispute (no co-located miss); F2 co-locates with a missed
        // key row. Despite F1 appearing first in `disputed`, the recoverable F2
        // must lead the queue.
        let grade = GradeResult {
            matched: Vec::new(),
            disputed: vec![
                cand("a.rs", 5, "security", "F1"),
                cand("b.rs", 10, "security", "F2"),
            ],
            missed: vec![key("b.rs", 10, "runtime-crash")],
        };
        let q = build_queue(&grade);
        assert_eq!(q.items.len(), 2);
        assert_eq!(q.items[0].finding_id, "F2", "recoverable item leads");
        assert!(q.items[0].is_recoverable());
        assert_eq!(q.items[0].recoverable_against.len(), 1);
        assert_eq!(q.items[0].recoverable_against[0].category, "runtime-crash");
        assert_eq!(q.items[1].finding_id, "F1");
        assert!(!q.items[1].is_recoverable());
    }

    #[test]
    fn build_queue_summary_counts_partition_and_only_queues_disputes() {
        let grade = GradeResult {
            matched: vec![Match {
                candidate: cand("m.rs", 1, "security", "F0"),
                key: key("m.rs", 1, "security"),
            }],
            disputed: vec![cand("b.rs", 10, "security", "F2")],
            missed: vec![
                key("b.rs", 10, "runtime-crash"), // recoverable (co-located with F2)
                key("z.rs", 99, "perf"),          // genuine miss, no co-located dispute
            ],
        };
        let q = build_queue(&grade);
        assert_eq!(q.summary.matched, 1);
        assert_eq!(q.summary.disputed, 1);
        assert_eq!(q.summary.missed, 2);
        assert_eq!(
            q.summary.recoverable_misses, 1,
            "only b.rs:10 is recoverable"
        );
        // The matched candidate and the genuine miss are summarized, not queued.
        assert_eq!(q.items.len(), 1);
        assert_eq!(q.items[0].finding_id, "F2");
    }

    #[test]
    fn item_id_falls_back_to_position_when_source_id_absent() {
        let mut idless = cand("a.rs", 5, "security", "");
        idless.source_id = None;
        let grade = GradeResult {
            matched: Vec::new(),
            disputed: vec![idless],
            missed: Vec::new(),
        };
        let q = build_queue(&grade);
        assert_eq!(q.items[0].finding_id, "item-0");
    }

    #[test]
    fn build_queue_disambiguates_findings_that_share_a_source_id() {
        // Two distinct findings can carry the SAME source id. They must not
        // collapse to one label target: the repeat is suffixed with its position
        // so each id resolves to its OWN finding, never silently to the first.
        let grade = GradeResult {
            matched: Vec::new(),
            disputed: vec![
                cand("a.rs", 5, "security", "F1"),
                cand("b.rs", 9, "perf", "F1"),
            ],
            missed: Vec::new(),
        };
        let q = build_queue(&grade);
        assert_eq!(q.items.len(), 2);
        assert_eq!(
            q.items[0].finding_id, "F1",
            "the first use keeps the bare id"
        );
        assert_ne!(
            q.items[1].finding_id, q.items[0].finding_id,
            "the colliding second finding gets a distinct id"
        );
        // Each id binds to its own finding — the collision-to-first-match bug.
        assert_eq!(
            q.item(&q.items[0].finding_id).unwrap().candidate.file,
            "a.rs"
        );
        assert_eq!(
            q.item(&q.items[1].finding_id).unwrap().candidate.file,
            "b.rs"
        );
    }

    #[test]
    fn apply_label_mints_label_tracing_to_the_finding() {
        let grade = GradeResult {
            matched: Vec::new(),
            disputed: vec![cand("a.rs", 5, "security", "F1")],
            missed: Vec::new(),
        };
        let q = build_queue(&grade);
        let label = apply_label(
            &q.items[0],
            Verdict::Keep,
            Disposition { in_scope: true },
            &LabelConditions {
                latency_ms: 1200,
                saw_grader_before_commit: false,
                timestamp: "2026-06-29T12:00:00Z".to_string(),
            },
        );
        assert_eq!(label.finding_id, "F1", "label traces to its source finding");
        assert_eq!(label.verdict, Verdict::Keep);
        assert!(label.disposition.in_scope);
        assert_eq!(label.latency_ms, 1200);
        assert_eq!(label.schema_version, LABEL_SCHEMA);
    }

    #[test]
    fn queue_item_lookup_finds_known_and_rejects_unknown() {
        let q = build_queue(&GradeResult {
            matched: Vec::new(),
            disputed: vec![cand("a.rs", 5, "security", "F1")],
            missed: Vec::new(),
        });
        assert!(q.item("F1").is_some());
        assert!(q.item("F404").is_none(), "an unknown id is not an item");
    }

    #[test]
    fn labeled_queue_round_trips_as_the_schema_stamped_artifact() {
        // End-to-end: build a queue, clear it by applying one label per item, and
        // round-trip the labeled artifact — the contract 005/002.5 consume.
        let grade = GradeResult {
            matched: Vec::new(),
            disputed: vec![
                cand("a.rs", 5, "security", "F1"),
                cand("b.rs", 10, "security", "F2"),
                cand("c.rs", 20, "perf", "F3"),
                cand("d.rs", 30, "style", "F4"),
            ],
            missed: vec![key("b.rs", 10, "runtime-crash")],
        };
        let mut q = build_queue(&grade);
        assert_eq!(q.items.len(), 4);

        let decisions = [
            (Verdict::Keep, true),
            (Verdict::Nit, true),
            (Verdict::Wrong, false),
            (Verdict::Noise, false),
        ];
        let items = q.items.clone();
        for (item, (verdict, in_scope)) in items.iter().zip(decisions) {
            q.labels.push(apply_label(
                item,
                verdict,
                Disposition { in_scope },
                &LabelConditions::default(),
            ));
        }
        assert_eq!(q.labels.len(), 4);

        let json = serde_json::to_string(&q).unwrap();
        assert!(
            json.contains(r#""schema_version":"crucible.judgment_queue.v1""#),
            "{json}"
        );
        let back: JudgmentQueue = serde_json::from_str(&json).unwrap();
        assert_eq!(q, back, "the labeled queue round-trips intact");
    }

    #[test]
    fn unlabeled_queue_omits_labels_and_defaults_schema_on_load() {
        let q = build_queue(&GradeResult {
            matched: Vec::new(),
            disputed: vec![cand("a.rs", 5, "security", "F1")],
            missed: Vec::new(),
        });
        let json = serde_json::to_string(&q).unwrap();
        assert!(
            !json.contains("labels"),
            "an empty labels vec is skipped on the wire: {json}"
        );

        // A queue object predating the schema field still loads with the default,
        // and a candidate without a source id loads as None.
        let minimal = r#"{
            "summary": {"matched":0,"disputed":1,"missed":0,"recoverable_misses":0},
            "items": [{
                "finding_id": "F1",
                "candidate": {"file":"a.rs","line":5,"category":"security","severity":"blocking","description":"d"}
            }]
        }"#;
        let loaded: JudgmentQueue = serde_json::from_str(minimal).unwrap();
        assert_eq!(loaded.schema_version, JUDGMENT_QUEUE_SCHEMA);
        assert!(loaded.labels.is_empty());
        assert!(loaded.items[0].candidate.source_id.is_none());
    }

    #[test]
    fn unknown_queue_schema_version_is_rejected() {
        // A present-but-unknown schema_version fails to load rather than being
        // silently treated as v1; an absent one still defaults (above).
        let json = r#"{
            "schema_version": "crucible.judgment_queue.v999",
            "summary": {"matched":0,"disputed":0,"missed":0,"recoverable_misses":0},
            "items": []
        }"#;
        let err = serde_json::from_str::<JudgmentQueue>(json).unwrap_err();
        assert!(
            err.to_string().contains("schema_version"),
            "error should name the bad schema_version: {err}"
        );
    }

    // ---- reconcile_labels (append-only correction collapse) ---------------

    fn label(fid: &str, verdict: Verdict, in_scope: bool, timestamp: &str) -> Label {
        Label {
            schema_version: LABEL_SCHEMA.to_string(),
            finding_id: fid.to_string(),
            verdict,
            disposition: Disposition { in_scope },
            latency_ms: 0,
            saw_grader_before_commit: false,
            timestamp: timestamp.to_string(),
        }
    }

    #[test]
    fn reconcile_labels_honors_the_latest_correction() {
        // A retracted ACCEPT: a later-timestamp correction for the same finding
        // overrides the original, so only the in-force ruling survives. Without
        // this, the stale ACCEPT would still extend the key and bump the version.
        let labels = vec![
            label("F1", Verdict::Keep, true, "2026-06-29T10:00:00Z"),
            label("F1", Verdict::Noise, false, "2026-06-29T12:00:00Z"),
        ];
        let reconciled = reconcile_labels(&labels);
        assert_eq!(reconciled.len(), 1, "one finding -> one effective label");
        assert_eq!(
            reconciled[0].verdict,
            Verdict::Noise,
            "the latest correction wins"
        );
        assert!(!reconciled[0].disposition.in_scope);
    }

    #[test]
    fn reconcile_labels_collapses_duplicates_and_keeps_first_seen_order() {
        // A duplicated decision counts once; distinct findings keep the order they
        // were first committed in, so the reconciled history renders stably.
        let labels = vec![
            label("F3", Verdict::Keep, true, "2026-06-29T10:00:00Z"),
            label("F1", Verdict::Keep, true, "2026-06-29T10:00:00Z"),
            label("F3", Verdict::Keep, true, "2026-06-29T11:00:00Z"), // duplicate of F3
        ];
        let reconciled = reconcile_labels(&labels);
        assert_eq!(reconciled.len(), 2, "F3 collapses to one; F1 stands");
        assert_eq!(reconciled[0].finding_id, "F3", "first-seen order preserved");
        assert_eq!(reconciled[1].finding_id, "F1");
    }

    #[test]
    fn reconcile_labels_breaks_equal_timestamp_ties_by_last_appended() {
        // Equal (here empty) timestamps fall to the last-appended label — the
        // append-only correction overwrites even when the clock is unrecorded.
        let labels = vec![
            label("F1", Verdict::Keep, true, ""),
            label("F1", Verdict::Wrong, false, ""),
        ];
        let reconciled = reconcile_labels(&labels);
        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].verdict, Verdict::Wrong, "last appended wins");
    }
}
