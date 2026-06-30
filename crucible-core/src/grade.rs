//! Deterministic pre-graders: the confident, machine-checkable backbone that
//! runs *before* any model or human judgment (backlog 002 child 3, epic 003).
//!
//! Three jobs, over findings already projected into [`KeyFinding`] rows by
//! [`crate::adapter`]:
//!
//! - [`schema_valid`] — is a Cerberus [`Finding`] structurally well-formed per
//!   the schema's *semantic* contract (an id, confidence in `[0, 1]`, a
//!   category, some content)? This is the validity serde cannot express: every
//!   field is already present after deserialization, but serde accepts an empty
//!   string or an out-of-range `f32`. The separate "anchor cites a real
//!   *changed* line" check needs the diff and is deliberately **not** folded in
//!   here — backlog 002.3 lists it as its own grader, and locatability is
//!   already handled at match time (an unlocatable row simply fails
//!   [`key_match`] and lands in `disputed`).
//! - [`dedup`] — collapse key rows that name the same finding by
//!   `(file, line, category)`; first occurrence wins, input order preserved.
//! - [`key_match`] / [`grade`] — partition a candidate review against an answer
//!   key into MATCHED (candidate agrees with a key row), DISPUTED (found, not in
//!   key), and MISSED (in key, not found). The predicate is deliberately strict
//!   so only confident agreement is auto-resolved; everything ambiguous flows to
//!   adjudication, which is the whole point of a pre-grader.
//!
//! The matcher is **greedy and order-sensitive**, not a maximum-cardinality
//! bipartite solver: each candidate claims the first still-unclaimed key it
//! matches. For the near-1:1 finding sets this eval grades that is the right
//! altitude; the pathological case (one candidate within tolerance of several
//! keys) is exactly the ambiguity the downstream model/human judge exists to
//! resolve, not something the deterministic floor should silently optimize.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::artifact::Finding;
use crate::key::KeyFinding;

/// Maximum line distance, in either direction, for two rows to be the "same"
/// location. Anchors drift by a line or two across diff context, rename, and
/// off-by-one reporting, so an exact line match is too brittle; `±2` absorbs
/// that without letting unrelated nearby findings collide.
pub const LINE_TOLERANCE: u32 = 2;

/// One MATCHED pair: a candidate row and the key row it satisfied.
///
/// Keeping both sides (rather than just the candidate) lets a downstream report
/// show *what* in the key a finding resolved, and lets a caller compare their
/// severities or descriptions without re-running the match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Match {
    /// The candidate review row that matched.
    pub candidate: KeyFinding,
    /// The answer-key row it was matched against.
    pub key: KeyFinding,
}

/// The deterministic grade of one candidate review against one answer key.
///
/// The three sets are disjoint and total over their sources: every candidate
/// row appears in exactly one of `matched`/`disputed`, every key row in exactly
/// one of `matched`/`missed`. Counts come straight from `.len()`, so a caller
/// can feed precision (`matched / (matched + disputed)`) and recall
/// (`matched / (matched + missed)`) into [`crate::measure`] without this type
/// pre-computing a rate it cannot attach an interval to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradeResult {
    /// Candidate rows that agreed with a key row, paired with that row.
    pub matched: Vec<Match>,
    /// Candidate rows with no matching key row — found, not in key.
    pub disputed: Vec<KeyFinding>,
    /// Key rows no candidate matched — in key, not found.
    pub missed: Vec<KeyFinding>,
}

/// Whether a Cerberus [`Finding`] is structurally well-formed enough to grade.
///
/// Checks the semantic contract serde cannot enforce on a value that already
/// deserialized:
///
/// - `id` is non-empty — findings are referenced by id (comments, fixes);
///   an unidentifiable finding is malformed.
/// - `confidence` is within `[0.0, 1.0]` — the documented range. `NaN` and
///   infinities fall outside it and are rejected.
/// - `category` is non-empty — [`key_match`] keys on it; a blank category
///   cannot be graded.
/// - `title` or `description` carries content — a finding must assert
///   *something*; both blank is malformed.
///
/// Deliberately **not** checked here: that an anchor cites a real changed line
/// (a distinct grader that needs the diff). An anchorless-but-otherwise-valid
/// finding is still schema-valid; it will simply fail to locate at match time.
pub fn schema_valid(finding: &Finding) -> bool {
    let has_id = !finding.id.trim().is_empty();
    let confidence_in_range = (0.0..=1.0).contains(&finding.confidence);
    let has_category = !finding.category.trim().is_empty();
    let has_content = !finding.title.trim().is_empty() || !finding.description.trim().is_empty();
    has_id && confidence_in_range && has_category && has_content
}

/// Drop key rows that repeat an earlier row's `(file, line, category)`.
///
/// Two rows that share that triple name the same finding location and class, so
/// the later one is redundant. The **first** occurrence is kept (with its own
/// severity and description) and input order is preserved, so dedup is a stable
/// shrink a caller can reason about. Comparison is exact: file paths are
/// case- and separator-sensitive, categories compared verbatim.
pub fn dedup(findings: Vec<KeyFinding>) -> Vec<KeyFinding> {
    let mut seen: HashSet<(String, u32, String)> = HashSet::with_capacity(findings.len());
    let mut out = Vec::with_capacity(findings.len());
    for f in findings {
        let id = (f.file.clone(), f.line, f.category.clone());
        if seen.insert(id) {
            out.push(f);
        }
    }
    out
}

/// Whether a candidate row matches an answer-key row.
///
/// All three axes must agree:
///
/// - **File**: exact, non-empty, repo-relative path equality. Requiring
///   non-empty rejects the adapter's fully-unanchored sentinel (`file == ""`)
///   so an unlocatable candidate never matches a real key.
/// - **Line**: locatable (`line != 0`) and within [`LINE_TOLERANCE`] in either
///   direction. The `line != 0` guard rejects the adapter's *line-less*
///   sentinel: a path-only `File` anchor — or a preferred `Inline`/`Change`
///   anchor that carries no line — projects to line `0`, meaning "this finding
///   names a file, not a specific line". Without the guard, `±LINE_TOLERANCE`
///   around `0` would let such a file-level finding spuriously match a real key
///   at line 1 or 2, silently inflating recall; with it, a line-less candidate
///   routes to `disputed` for the judge instead of auto-matching — consistent
///   with "only confident agreement is auto-resolved".
/// - **Category**: equal once normalized (case-folded, separators `-`/`_`/space
///   unified). This treats surface variants like `runtime-crash`,
///   `runtime_crash`, and `Runtime Crash` as the same class but does **not**
///   bridge genuinely different vocabularies (e.g. Cerberus `security` vs a
///   Daedalus `runtime-crash`) — cross-vocabulary synonymy is a semantic
///   judgment left to the model/human judge, not the deterministic floor. A
///   candidate that agrees on location but not category lands in `disputed`
///   (and its key row in `missed`); [`recoverable_misses`] re-surfaces those
///   location agreements so the recall is not read as final.
pub fn key_match(cand: &KeyFinding, key: &KeyFinding) -> bool {
    !cand.file.is_empty()
        && cand.line != 0
        && cand.file == key.file
        && cand.line.abs_diff(key.line) <= LINE_TOLERANCE
        && category_compatible(&cand.category, &key.category)
}

/// Grade a candidate review against an answer key.
///
/// Greedy, order-sensitive matching: each candidate, in order, claims the first
/// still-unclaimed key row it [`key_match`]es. Unmatched candidates are
/// `disputed`; unclaimed keys are `missed`. The candidate and key lists are
/// taken as already prepared — callers should map via [`crate::adapter`], drop
/// invalid rows with [`schema_valid`], and [`dedup`] the key first; `grade`
/// itself does no filtering, so its output is a pure function of its inputs.
pub fn grade(cand: &[KeyFinding], key: &[KeyFinding]) -> GradeResult {
    let mut matched = Vec::new();
    let mut disputed = Vec::new();
    let mut claimed = vec![false; key.len()];

    for c in cand {
        let hit = key
            .iter()
            .enumerate()
            .find(|(i, k)| !claimed[*i] && key_match(c, k))
            .map(|(i, _)| i);
        match hit {
            Some(i) => {
                claimed[i] = true;
                matched.push(Match {
                    candidate: c.clone(),
                    key: key[i].clone(),
                });
            }
            None => disputed.push(c.clone()),
        }
    }

    let missed = key
        .iter()
        .enumerate()
        .filter(|(i, _)| !claimed[*i])
        .map(|(_, k)| k.clone())
        .collect();

    GradeResult {
        matched,
        disputed,
        missed,
    }
}

/// How many `missed` key rows a `disputed` candidate agrees with on *location*
/// but not category — the correct-location findings the category-strict
/// [`key_match`] routed into `missed` instead of `matched`.
///
/// Cerberus and Daedalus use overlapping-but-distinct category vocabularies, so
/// a correct finding at the right `file:line` labeled with a Daedalus-foreign
/// or differently-named category fails [`key_match`] and double-penalizes the
/// grade: the candidate becomes a `disputed` false positive **and** its key row
/// a `missed` false negative. Resolving that cross-vocabulary synonymy is a
/// semantic call this deterministic floor deliberately leaves to the judge, so
/// `grade` does not auto-match across vocabularies. But it must not let a recall
/// computed over the deflated `missed` set read as final: this count is exactly
/// the misses a judge could still recover. A reporter should surface it so
/// `matched / (matched + missed)` is understood as a category-strict
/// pre-adjudication floor — the true recall is at least that and at most the
/// rate with these misses recovered.
///
/// Counted from the `missed` side: a key row counts once if **any** disputed
/// candidate shares its location, regardless of how many do. Location agreement
/// uses the same locatable-and-within-[`LINE_TOLERANCE`] rule as [`key_match`],
/// so the line-less / unanchored sentinels (`line == 0`, empty `file`) never
/// count as agreement.
pub fn recoverable_misses(result: &GradeResult) -> usize {
    result
        .missed
        .iter()
        .filter(|m| result.disputed.iter().any(|d| location_agrees(d, m)))
        .count()
}

/// Whether two rows name the same location: same non-empty `file`, both lines
/// locatable (`!= 0`), within [`LINE_TOLERANCE`]. The category-free half of
/// [`key_match`]'s predicate, reused so "location agreement" means the same
/// thing in both places.
fn location_agrees(a: &KeyFinding, b: &KeyFinding) -> bool {
    !a.file.is_empty()
        && a.file == b.file
        && a.line != 0
        && b.line != 0
        && a.line.abs_diff(b.line) <= LINE_TOLERANCE
}

/// Whether two category strings name the same class after normalization.
///
/// Empty-after-normalization inputs never match, so a blank category cannot
/// silently pair with another blank.
fn category_compatible(a: &str, b: &str) -> bool {
    let na = normalize_category(a);
    !na.is_empty() && na == normalize_category(b)
}

/// Case-fold and collapse `-`/`_`/whitespace runs to single spaces, trimmed.
///
/// `"Runtime-Crash"`, `"runtime_crash"`, and `" runtime  crash "` all normalize
/// to `"runtime crash"`.
fn normalize_category(c: &str) -> String {
    let mut out = String::with_capacity(c.len());
    let mut prev_sep = true; // leading position: suppress separators
    for ch in c.chars() {
        if ch == '-' || ch == '_' || ch.is_whitespace() {
            if !prev_sep {
                out.push(' ');
                prev_sep = true;
            }
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Severity;

    /// A Cerberus finding with the fields `schema_valid` reads; the rest filler.
    fn finding(
        id: &str,
        confidence: f32,
        category: &str,
        title: &str,
        description: &str,
    ) -> Finding {
        Finding {
            id: id.to_string(),
            severity: Severity::Minor,
            category: category.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            evidence: "e".to_string(),
            confidence,
            anchors: Vec::new(),
            citations: Vec::new(),
            suggested_fixes: Vec::new(),
        }
    }

    /// A well-formed baseline finding; individual tests mutate one field.
    fn valid_finding() -> Finding {
        finding("F1", 0.8, "security", "Headline", "Body")
    }

    fn kf(file: &str, line: u32, category: &str) -> KeyFinding {
        KeyFinding {
            file: file.to_string(),
            line,
            category: category.to_string(),
            severity: "blocking".to_string(),
            description: "d".to_string(),
        }
    }

    // ---- schema_valid -----------------------------------------------------

    #[test]
    fn schema_valid_accepts_well_formed_finding() {
        assert!(schema_valid(&valid_finding()));
    }

    #[test]
    fn schema_valid_accepts_finding_with_only_a_title() {
        let mut f = valid_finding();
        f.description = "   ".to_string();
        assert!(schema_valid(&f), "a title alone is enough content");
    }

    #[test]
    fn schema_valid_accepts_anchorless_finding() {
        // Locatability is a match-time concern, not a schema-validity one.
        let f = valid_finding();
        assert!(f.anchors.is_empty());
        assert!(schema_valid(&f));
    }

    #[test]
    fn schema_valid_rejects_empty_id() {
        let mut f = valid_finding();
        f.id = "  ".to_string();
        assert!(!schema_valid(&f));
    }

    #[test]
    fn schema_valid_rejects_confidence_out_of_range() {
        let mut f = valid_finding();
        f.confidence = 1.5;
        assert!(!schema_valid(&f), "confidence above 1.0 is invalid");
        f.confidence = -0.1;
        assert!(!schema_valid(&f), "negative confidence is invalid");
    }

    #[test]
    fn schema_valid_rejects_nan_confidence() {
        let mut f = valid_finding();
        f.confidence = f32::NAN;
        assert!(!schema_valid(&f), "NaN is outside [0,1]");
    }

    #[test]
    fn schema_valid_accepts_confidence_at_bounds() {
        let mut f = valid_finding();
        f.confidence = 0.0;
        assert!(schema_valid(&f));
        f.confidence = 1.0;
        assert!(schema_valid(&f));
    }

    #[test]
    fn schema_valid_rejects_empty_category() {
        let mut f = valid_finding();
        f.category = String::new();
        assert!(!schema_valid(&f));
    }

    #[test]
    fn schema_valid_rejects_finding_with_no_content() {
        let mut f = valid_finding();
        f.title = "  ".to_string();
        f.description = String::new();
        assert!(
            !schema_valid(&f),
            "blank title and description says nothing"
        );
    }

    // ---- dedup ------------------------------------------------------------

    #[test]
    fn dedup_keeps_distinct_rows_in_order() {
        let rows = vec![kf("a.rs", 1, "x"), kf("b.rs", 2, "y"), kf("c.rs", 3, "z")];
        let out = dedup(rows.clone());
        assert_eq!(out, rows, "no duplicates: unchanged and ordered");
    }

    #[test]
    fn dedup_drops_exact_triple_duplicate_keeping_first() {
        let mut first = kf("a.rs", 10, "security");
        first.description = "first".to_string();
        let mut second = kf("a.rs", 10, "security");
        second.description = "second".to_string();
        let out = dedup(vec![first.clone(), second]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].description, "first", "first occurrence wins");
    }

    #[test]
    fn dedup_distinguishes_on_each_axis() {
        let rows = vec![
            kf("a.rs", 10, "security"),
            kf("a.rs", 10, "perf"),     // category differs
            kf("a.rs", 11, "security"), // line differs
            kf("b.rs", 10, "security"), // file differs
        ];
        assert_eq!(
            dedup(rows.clone()),
            rows,
            "any axis differing keeps the row"
        );
    }

    #[test]
    fn dedup_of_empty_is_empty() {
        assert!(dedup(Vec::new()).is_empty());
    }

    // ---- key_match --------------------------------------------------------

    #[test]
    fn key_match_exact() {
        assert!(key_match(
            &kf("a.rs", 10, "security"),
            &kf("a.rs", 10, "security")
        ));
    }

    #[test]
    fn key_match_within_line_tolerance() {
        let key = kf("a.rs", 10, "security");
        for line in [8, 9, 10, 11, 12] {
            assert!(
                key_match(&kf("a.rs", line, "security"), &key),
                "line {line} within ±2"
            );
        }
    }

    #[test]
    fn key_match_outside_line_tolerance() {
        let key = kf("a.rs", 10, "security");
        assert!(
            !key_match(&kf("a.rs", 13, "security"), &key),
            "+3 is too far"
        );
        assert!(
            !key_match(&kf("a.rs", 7, "security"), &key),
            "-3 is too far"
        );
    }

    #[test]
    fn key_match_requires_same_file() {
        assert!(!key_match(
            &kf("a.rs", 10, "security"),
            &kf("b.rs", 10, "security")
        ));
    }

    #[test]
    fn key_match_rejects_empty_candidate_file() {
        // The adapter's unanchored sentinel must never match a real key.
        assert!(!key_match(&kf("", 0, "security"), &kf("", 0, "security")));
    }

    #[test]
    fn key_match_rejects_line_zero_candidate_sentinel() {
        // A file-level finding (path-only anchor, or a line-less inline/change
        // anchor) projects to line 0 — it names a file, not a line. The
        // ±tolerance window around 0 must NOT let it match a real key at line 1
        // or 2; such a candidate is unlocated and routes to `disputed`.
        let cand = kf("a.rs", 0, "security"); // file set, line-less sentinel
        assert!(
            !key_match(&cand, &kf("a.rs", 1, "security")),
            "line-0 sentinel must not match a key at line 1"
        );
        assert!(
            !key_match(&cand, &kf("a.rs", 2, "security")),
            "line-0 sentinel must not match a key at line 2"
        );
        assert!(
            !key_match(&cand, &kf("a.rs", 0, "security")),
            "two line-0 rows are not a confident location agreement"
        );
        // A genuinely located candidate at the same file still matches.
        assert!(key_match(
            &kf("a.rs", 1, "security"),
            &kf("a.rs", 1, "security")
        ));
    }

    #[test]
    fn key_match_category_is_case_and_separator_insensitive() {
        let key = kf("a.rs", 10, "runtime-crash");
        assert!(key_match(&kf("a.rs", 10, "Runtime-Crash"), &key));
        assert!(key_match(&kf("a.rs", 10, "runtime_crash"), &key));
        assert!(key_match(&kf("a.rs", 10, "runtime crash"), &key));
    }

    #[test]
    fn key_match_rejects_different_category() {
        assert!(!key_match(
            &kf("a.rs", 10, "security"),
            &kf("a.rs", 10, "runtime-crash")
        ));
    }

    // ---- grade ------------------------------------------------------------

    #[test]
    fn grade_of_empty_inputs_is_empty() {
        let r = grade(&[], &[]);
        assert!(r.matched.is_empty() && r.disputed.is_empty() && r.missed.is_empty());
    }

    #[test]
    fn grade_single_match() {
        let cand = vec![kf("a.rs", 10, "security")];
        let key = vec![kf("a.rs", 11, "security")]; // within tolerance
        let r = grade(&cand, &key);
        assert_eq!(r.matched.len(), 1);
        assert!(r.disputed.is_empty());
        assert!(r.missed.is_empty());
        assert_eq!(r.matched[0].candidate, cand[0]);
        assert_eq!(r.matched[0].key, key[0]);
    }

    #[test]
    fn grade_unmatched_candidate_is_disputed() {
        let cand = vec![kf("a.rs", 10, "security")];
        let r = grade(&cand, &[]);
        assert_eq!(r.disputed, cand);
        assert!(r.matched.is_empty() && r.missed.is_empty());
    }

    #[test]
    fn grade_unmatched_key_is_missed() {
        let key = vec![kf("a.rs", 10, "security")];
        let r = grade(&[], &key);
        assert_eq!(r.missed, key);
        assert!(r.matched.is_empty() && r.disputed.is_empty());
    }

    #[test]
    fn grade_mixed_match_and_dispute_and_miss() {
        let cand = vec![
            kf("a.rs", 10, "security"), // matches key[0]
            kf("z.rs", 99, "perf"),     // matches nothing -> disputed
        ];
        let key = vec![
            kf("a.rs", 10, "security"),    // matched by cand[0]
            kf("b.rs", 20, "correctness"), // found by nobody -> missed
        ];
        let r = grade(&cand, &key);
        assert_eq!(r.matched.len(), 1);
        assert_eq!(r.matched[0].candidate, cand[0]);
        assert_eq!(r.disputed, vec![cand[1].clone()]);
        assert_eq!(r.missed, vec![key[1].clone()]);
    }

    #[test]
    fn grade_each_key_is_claimed_at_most_once() {
        // Two candidates both match the one key; only the first wins, the
        // second is disputed (greedy, order-sensitive).
        let cand = vec![kf("a.rs", 10, "security"), kf("a.rs", 11, "security")];
        let key = vec![kf("a.rs", 10, "security")];
        let r = grade(&cand, &key);
        assert_eq!(r.matched.len(), 1);
        assert_eq!(
            r.matched[0].candidate, cand[0],
            "first candidate claims the key"
        );
        assert_eq!(r.disputed, vec![cand[1].clone()]);
        assert!(r.missed.is_empty());
    }

    #[test]
    fn grade_candidate_claims_first_of_several_matchable_keys() {
        // One candidate within tolerance of two keys claims the first; the
        // second key is missed. Documents the greedy choice.
        let cand = vec![kf("a.rs", 10, "security")];
        let key = vec![kf("a.rs", 10, "security"), kf("a.rs", 12, "security")];
        let r = grade(&cand, &key);
        assert_eq!(r.matched.len(), 1);
        assert_eq!(r.matched[0].key, key[0]);
        assert_eq!(r.missed, vec![key[1].clone()]);
        assert!(r.disputed.is_empty());
    }

    // ---- recoverable_misses ----------------------------------------------

    #[test]
    fn recoverable_misses_of_empty_grade_is_zero() {
        let r = grade(&[], &[]);
        assert_eq!(recoverable_misses(&r), 0);
    }

    #[test]
    fn recoverable_misses_counts_colocated_category_mismatch() {
        // Same file:line, different category vocabulary: the candidate is
        // disputed and the key missed, but they agree on *where* — exactly the
        // miss a judge could recover.
        let cand = vec![kf("a.rs", 10, "security")];
        let key = vec![kf("a.rs", 10, "runtime-crash")];
        let r = grade(&cand, &key);
        assert_eq!(r.matched.len(), 0, "category mismatch blocks the match");
        assert_eq!(r.disputed.len(), 1);
        assert_eq!(r.missed.len(), 1);
        assert_eq!(
            recoverable_misses(&r),
            1,
            "the co-located miss is recoverable"
        );
    }

    #[test]
    fn recoverable_misses_within_line_tolerance() {
        // Location agreement uses the same ±LINE_TOLERANCE rule as key_match.
        let cand = vec![kf("a.rs", 10, "security")];
        let key = vec![kf("a.rs", 12, "runtime-crash")]; // within ±2
        let r = grade(&cand, &key);
        assert_eq!(recoverable_misses(&r), 1);
    }

    #[test]
    fn recoverable_misses_ignores_different_file_or_far_line() {
        let cand = vec![kf("a.rs", 10, "security"), kf("a.rs", 10, "perf")];
        let key = vec![
            kf("b.rs", 10, "runtime-crash"), // different file
            kf("a.rs", 20, "runtime-crash"), // line too far
        ];
        let r = grade(&cand, &key);
        assert_eq!(r.matched.len(), 0);
        assert_eq!(
            recoverable_misses(&r),
            0,
            "no co-located disputed candidate"
        );
    }

    #[test]
    fn recoverable_misses_excludes_line_zero_and_unanchored() {
        // A line-less (line 0) or unanchored ("") disputed candidate is not a
        // confident location, so it never makes a miss "recoverable".
        let cand = vec![kf("a.rs", 0, "security"), kf("", 0, "security")];
        let key = vec![kf("a.rs", 1, "runtime-crash")];
        let r = grade(&cand, &key);
        assert_eq!(recoverable_misses(&r), 0);
    }

    #[test]
    fn recoverable_misses_does_not_count_a_matched_key() {
        // Matched keys are not in `missed`, so they cannot be recoverable; only
        // the genuinely category-blocked miss counts.
        let cand = vec![kf("a.rs", 10, "security"), kf("b.rs", 5, "security")];
        let key = vec![
            kf("a.rs", 10, "security"),     // matched by cand[0]
            kf("b.rs", 5, "runtime-crash"), // co-located with cand[1], blocked
        ];
        let r = grade(&cand, &key);
        assert_eq!(r.matched.len(), 1);
        assert_eq!(recoverable_misses(&r), 1);
    }
}
