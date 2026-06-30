//! Export adjudicated findings into the Daedalus answer-key artifacts
//! (backlog 002 child 5 — the eval-improvement flywheel's write side).
//!
//! The grade ([`mod@crate::grade`]) and the adjudication queue ([`crate::judgment`])
//! are the read side: they take a review and a key and surface what a judge must
//! rule on. This module is the write side — it takes the [`Label`]s
//! a judge committed (carried on a [`JudgmentQueue`]) and emits three artifacts,
//! one human and two machine:
//!
//! - an [`adjudications.md`](render_adjudications_md) key-extension **log**, in
//!   the real `arenas/<id>/adjudications.md` shape (title, doctrine preamble, a
//!   `| id | date | task | finding | ruling |` table, and a per-finding `##`
//!   section). This is a Crucible-authored **human record**; Daedalus has no
//!   parser for it.
//! - where a finding is accepted, the [`extended_key`] —
//!   `solution/findings.json` ([`AnswerKey`]) with the accepted findings added —
//!   the human-readable point **oracle** the ACCEPT doctrine extends, and
//! - the [`extended_expected_key`] — `tests/expected.json` ([`ExpectedKey`])
//!   with each accepted finding added as a line-span [`Defect`].
//!   This is the **machine scorer key** `daedalus-score` actually reads (daedalus
//!   `score.rs`), so it is the one cross-repo contract that makes an accepted
//!   finding re-score as a true positive instead of a false positive — the whole
//!   point of the flywheel.
//!
//! ## The two real rulings, driven off [`Verdict`] + [`Disposition`]
//!
//! Daedalus's adjudication doctrine (DESIGN.md, *Adjudication*) has exactly two
//! outcomes, and this module derives each from the orthogonal correctness/scope
//! axes Crucible already records:
//!
//! - **ACCEPT** — the reviewer was right and the key was incomplete: extend the
//!   key *and oracle solution*, and bump the arena version (prior cross-version
//!   averaging becomes invalid; baselines re-run before any new comparison).
//!   Derived iff the judge ruled the finding correct-and-keepable *and* inside
//!   the change's declared contract: `verdict == Keep && disposition.in_scope`.
//! - **OUT-OF-SCOPE** — the key stands, only a rationale is recorded. Every
//!   other `(verdict, scope)` combination lands here: the canonical
//!   correct-but-out-of-contract finding (`Keep` + `!in_scope`, the real ADJ-2),
//!   a trivial nit, or a finding the judge confirmed [`Wrong`](Verdict::Wrong) /
//!   [`Noise`](Verdict::Noise) — in each case the deterministic key already
//!   excludes it correctly, so it must not change.
//!
//! A key change requires a version bump, so each ACCEPT walks the arena version
//! forward by one minor (resetting patch): a batch with two accepts goes
//! `0.2.0 → 0.3.0 → 0.4.0`, and each ACCEPT row records its own `from → to` pair.
//! OUT-OF-SCOPE rulings leave the version untouched.
//!
//! ## Round-trip (Crucible-internal serialization)
//!
//! [`render_adjudications_md`] and [`parse_adjudications_md`] are mutual inverses
//! over [`Adjudication`]: the per-`##` section carries every field a judgment
//! needs (finding id, location, verdict, disposition, ruling, version bump, the
//! escaped claim, and the calibration conditions), so an `adjudications.md`
//! Crucible wrote parses back into the exact adjudication set that produced it.
//! The summary table is a human-readable view regenerated on render and ignored
//! on parse — the sections are the source of truth.
//!
//! This round-trip is **Crucible-internal**: Crucible both writes the log and
//! reads it back (e.g. to re-derive judgments from a committed file). It is *not*
//! a cross-repo API — Daedalus has no `adjudications.md` parser. The machine
//! contract Daedalus consumes is the extended `tests/expected.json` its scorer
//! reads ([`extended_expected_key`]); the log is the human audit trail beside it.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use crate::judgment::{reconcile_labels, JudgmentQueue};
use crate::key::{AnswerKey, Defect, ExpectedKey, KeyFinding};
use crate::label::Label;
use crate::{Disposition, Verdict};

/// Title line prefix of an `adjudications.md`, shared by render and parse. The
/// arena id follows the em-dash separator.
const TITLE_PREFIX: &str = "# Answer-key adjudications — ";

/// Separator between an `##` section's id and the rest of its heading.
const HEADING_SEP: &str = " — ";

/// The standing doctrine paragraph every `adjudications.md` opens with, adapted
/// from Daedalus's real log so the rendered artifact reads in the same voice. A
/// close paraphrase, not byte-identical: the doctrine and the two rulings match,
/// the wording is Crucible's.
const PREAMBLE: &str = "\
The standing workflow for \"the candidate reported a finding the answer key does
not list\" (DESIGN.md, Adjudication): each disputed finding is adjudicated here,
then either **ACCEPT** — extend the key and oracle solution, bump the arena
version (prior cross-version averaging becomes invalid; baselines re-run before
any new comparison) — or **OUT-OF-SCOPE** — record the rationale and leave the
key unchanged. Keys improve instead of silently punishing reviewers better than
their author.
";

/// Max width of the table's `finding` cell before it is truncated; the cell is a
/// human summary, never parsed, so truncation loses nothing recoverable.
const FINDING_CELL_WIDTH: usize = 68;

/// A failure to build, render, or parse an adjudication artifact.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// A committed [`Label`] referenced a `finding_id` with no matching queue
    /// item, so its disputed finding — the location, category, and claim the
    /// adjudication renders — could not be recovered. In the normal flow labels
    /// are minted from queue items ([`apply_label`](crate::apply_label)), so this
    /// is a corrupted or mismatched input, not a routine outcome.
    #[error("label references finding id {finding_id:?}, which is not an item in this queue")]
    DanglingLabel {
        /// The offending label's finding id.
        finding_id: String,
    },

    /// An `adjudications.md` could not be parsed back into adjudications. The
    /// detail names the specific shape that was missing or malformed.
    #[error("malformed adjudications.md: {detail}")]
    Parse {
        /// What was missing or malformed.
        detail: String,
    },

    /// A string that should be a `major.minor.patch` arena version was not.
    /// Distinct from [`Parse`](ExportError::Parse) so a bad `--base-version`
    /// flag surfaces as an invalid version, not a misleading "malformed
    /// adjudications.md".
    #[error("invalid arena version {value:?}: {detail}")]
    Version {
        /// The offending version string.
        value: String,
        /// Why it is not a valid `major.minor.patch`.
        detail: String,
    },

    /// An ACCEPT's minor bump would overflow `u32` — a key cannot be extended
    /// past `major.4294967295.patch`. Caught rather than silently wrapping (or
    /// panicking in debug).
    #[error("arena version {version} cannot bump minor: u32 overflow")]
    VersionOverflow {
        /// The version whose minor component cannot advance.
        version: ArenaVersion,
    },
}

impl ExportError {
    fn parse(detail: impl Into<String>) -> Self {
        ExportError::Parse {
            detail: detail.into(),
        }
    }
}

/// A `major.minor.patch` arena version (DESIGN.md frozen-surface contract).
///
/// Only the operation the adjudication doctrine needs is modeled: [`bump_minor`]
/// for the ACCEPT key change. Parsing is exact — three dot-separated unsigned
/// integers and nothing else — so a malformed `--base-version` fails loudly
/// rather than silently truncating.
///
/// [`bump_minor`]: ArenaVersion::bump_minor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaVersion {
    /// Major component.
    pub major: u32,
    /// Minor component.
    pub minor: u32,
    /// Patch component.
    pub patch: u32,
}

impl ArenaVersion {
    /// The version one minor ahead, with patch reset to `0` — the bump a single
    /// ACCEPT applies (`0.2.1 → 0.3.0`).
    ///
    /// Errors with [`ExportError::VersionOverflow`] rather than wrapping (in
    /// release) or panicking (in debug) when `minor` already sits at the `u32`
    /// ceiling.
    pub fn bump_minor(self) -> Result<Self, ExportError> {
        let minor = self
            .minor
            .checked_add(1)
            .ok_or(ExportError::VersionOverflow { version: self })?;
        Ok(ArenaVersion {
            major: self.major,
            minor,
            patch: 0,
        })
    }
}

impl fmt::Display for ArenaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for ArenaVersion {
    type Err = ExportError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || ExportError::Version {
            value: s.to_string(),
            detail: "expected three dot-separated u32 components (major.minor.patch)".to_string(),
        };
        let parts: Vec<&str> = s.split('.').collect();
        let [major, minor, patch] = parts.as_slice() else {
            return Err(invalid());
        };
        Ok(ArenaVersion {
            major: major.parse().map_err(|_| invalid())?,
            minor: minor.parse().map_err(|_| invalid())?,
            patch: patch.parse().map_err(|_| invalid())?,
        })
    }
}

/// The ruling a judgment resolves to — the two outcomes Daedalus's doctrine
/// allows, derived from [`Verdict`] + [`Disposition`] (see the [module docs](self)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ruling {
    /// Extend the key and bump the arena version from `from` to `to`.
    Accept {
        /// The arena version before this acceptance.
        from: ArenaVersion,
        /// The arena version after it (one minor ahead).
        to: ArenaVersion,
    },
    /// Leave the key unchanged; only the rationale is recorded.
    OutOfScope,
}

impl Ruling {
    /// `ACCEPT` / `OUT-OF-SCOPE` — the bare label used in headings, the `Ruling`
    /// field, and the parenthetical at the end of a section heading.
    fn tag(&self) -> &'static str {
        match self {
            Ruling::Accept { .. } => "ACCEPT",
            Ruling::OutOfScope => "OUT-OF-SCOPE",
        }
    }

    /// The summary-table cell, including the version bump for an acceptance.
    /// Matches Daedalus's real wording exactly.
    fn table_cell(&self) -> String {
        match self {
            Ruling::Accept { from, to } => {
                format!("**ACCEPT** → key extended, arena {from} → {to}")
            }
            Ruling::OutOfScope => "**OUT-OF-SCOPE**".to_string(),
        }
    }

    /// Whether this ruling extends the key (and therefore bumps the version).
    fn is_accept(&self) -> bool {
        matches!(self, Ruling::Accept { .. })
    }
}

/// Whether a `(verdict, scope)` pair is an ACCEPT: the finding is correct and
/// worth keeping *and* falls inside the change's declared contract. Every other
/// combination is OUT-OF-SCOPE — the key stands. See the [module docs](self).
fn is_accept(verdict: Verdict, disposition: Disposition) -> bool {
    matches!(verdict, Verdict::Keep) && disposition.in_scope
}

/// One fully-resolved adjudication: a judge's decision on a disputed finding plus
/// everything needed to render it as — and recover it from — an `adjudications.md`
/// section.
///
/// Self-contained by design: it copies the disputed finding's location, category,
/// severity, and claim off the queue item rather than holding a reference, so the
/// rendered section carries every field and the parse is lossless. The source
/// finding id (`finding_id`) is the trace anchor back to the
/// [`Label`] and the review finding it judges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adjudication {
    /// The adjudication id within this log, `ADJ-1`, `ADJ-2`, … in queue order.
    pub id: String,
    /// Caller-supplied adjudication date (e.g. `2026-06-29`); may be empty.
    pub date: String,
    /// The Harbor task the finding was raised against (e.g. `py-file-cache`).
    pub task: String,
    /// The source finding id the judgment traces to (e.g. `F3`).
    pub finding_id: String,
    /// Repo-relative file of the disputed finding.
    pub file: String,
    /// Line the disputed finding is anchored to.
    pub line: u32,
    /// The disputed finding's category.
    pub category: String,
    /// The disputed finding's severity (may be empty for a severity-less row).
    pub severity: String,
    /// The disputed finding's claim/description, rendered escaped so it survives
    /// the single-line Markdown field round-trip intact.
    pub description: String,
    /// The judge's correctness verdict.
    pub verdict: Verdict,
    /// The judge's scope disposition, orthogonal to the verdict.
    pub disposition: Disposition,
    /// The derived ruling (carrying the version bump for an acceptance).
    pub ruling: Ruling,
    /// The calibration-validity conditions the judgment was committed under,
    /// carried so the round-trip recovers the full [`Label`].
    pub conditions: Conditions,
}

impl Adjudication {
    /// Whether this adjudication accepted the finding (and thus extends the key).
    pub fn is_accept(&self) -> bool {
        self.ruling.is_accept()
    }
}

/// The calibration conditions a judgment was committed under, mirroring the three
/// fields a [`Label`] carries beyond its verdict and scope.
///
/// A field-for-field copy of [`LabelConditions`](crate::LabelConditions), kept
/// local so `export` does not reach across into the judgment queue's input type;
/// [`adjudications_from_queue`] populates it straight off each label.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conditions {
    /// Milliseconds from card presentation to commit.
    pub latency_ms: u64,
    /// Whether the grader's verdict was visible before commit.
    pub saw_grader_before_commit: bool,
    /// Caller-supplied RFC 3339 commit timestamp; may be empty.
    pub timestamp: String,
}

/// The document-level context an adjudication run needs beyond the queue: which
/// arena and task the findings belong to, the date to stamp, and the arena
/// version the first acceptance bumps from.
#[derive(Debug, Clone)]
pub struct ExportContext {
    /// Arena id, e.g. `pr-review-v0` — the title and Harbor path component.
    pub arena: String,
    /// Harbor task id, e.g. `py-file-cache`.
    pub task: String,
    /// Date to stamp each adjudication with; may be empty.
    pub date: String,
    /// Arena version the first ACCEPT bumps from.
    pub base_version: ArenaVersion,
}

/// An `adjudications.md` parsed back into its arena and adjudications — the
/// inverse of [`render_adjudications_md`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAdjudications {
    /// The arena id from the title line.
    pub arena: String,
    /// The adjudications, in document order.
    pub adjudications: Vec<Adjudication>,
}

/// Turn a labeled [`JudgmentQueue`] into the ordered adjudications it implies.
///
/// The label history is first [`reconcile_labels`]d to the one effective label
/// per finding (latest correction wins), so a retracted ACCEPT does not still
/// extend the key or bump the version and a duplicated decision is not
/// double-counted. Each surviving label, in first-committed order, becomes one
/// [`Adjudication`]: its disputed finding is recovered from the matching queue
/// item, its ruling is derived from the label's verdict and disposition
/// (`is_accept`), and an acceptance walks the arena version one minor forward.
/// Ids are assigned `ADJ-1`, `ADJ-2`, … across both rulings, matching the real
/// log where accepts and out-of-scope entries share one sequence.
///
/// Returns [`ExportError::DanglingLabel`] if a label names a finding the queue
/// does not carry — the only way the disputed finding's claim could be missing —
/// or [`ExportError::VersionOverflow`] if an ACCEPT's minor bump overflows `u32`.
pub fn adjudications_from_queue(
    queue: &JudgmentQueue,
    ctx: &ExportContext,
) -> Result<Vec<Adjudication>, ExportError> {
    let mut version = ctx.base_version;
    let labels = reconcile_labels(&queue.labels);
    let mut out = Vec::with_capacity(labels.len());
    for label in labels {
        let item = queue
            .item(&label.finding_id)
            .ok_or_else(|| ExportError::DanglingLabel {
                finding_id: label.finding_id.clone(),
            })?;
        let ruling = if is_accept(label.verdict, label.disposition) {
            let from = version;
            let to = version.bump_minor()?;
            version = to;
            Ruling::Accept { from, to }
        } else {
            Ruling::OutOfScope
        };
        let cand = &item.candidate;
        out.push(Adjudication {
            id: format!("ADJ-{}", out.len() + 1),
            date: ctx.date.clone(),
            task: ctx.task.clone(),
            finding_id: label.finding_id.clone(),
            file: cand.file.clone(),
            line: cand.line,
            category: cand.category.clone(),
            severity: cand.severity.clone(),
            description: cand.description.clone(),
            verdict: label.verdict,
            disposition: label.disposition,
            ruling,
            conditions: conditions_of(label),
        });
    }
    Ok(out)
}

/// Copy a label's calibration conditions into the export's local [`Conditions`].
fn conditions_of(label: &Label) -> Conditions {
    Conditions {
        latency_ms: label.latency_ms,
        saw_grader_before_commit: label.saw_grader_before_commit,
        timestamp: label.timestamp.clone(),
    }
}

/// Extend an answer key with the accepted findings — the Harbor `solution/
/// findings.json` an ACCEPT produces.
///
/// The original rows are kept verbatim and each accepted adjudication is appended
/// as a [`KeyFinding`] (location, category, severity, and claim), with
/// `source_id` cleared because answer-key rows never carry one. Out-of-scope
/// adjudications leave the key unchanged, so a run with no acceptances returns
/// the key as-is — the doctrine's "the key stands".
///
/// **Idempotent by `(file, line, category)`.** An accept whose location and
/// category already match a row — an original key row or an earlier accept in the
/// same batch — is skipped rather than appended, so re-exporting the same
/// acceptance never grows a duplicate oracle row.
pub fn extended_key(original: &AnswerKey, adjudications: &[Adjudication]) -> AnswerKey {
    let mut findings = original.findings.clone();
    for adj in adjudications.iter().filter(|a| a.is_accept()) {
        if findings
            .iter()
            .any(|f| f.file == adj.file && f.line == adj.line && f.category == adj.category)
        {
            continue;
        }
        findings.push(KeyFinding {
            file: adj.file.clone(),
            line: adj.line,
            category: adj.category.clone(),
            severity: adj.severity.clone(),
            description: adj.description.clone(),
            source_id: None,
        });
    }
    AnswerKey { findings }
}

/// Extend a Daedalus **scorer** key (`tests/expected.json`) with the accepted
/// findings — the span key `daedalus-score` actually reads, distinct from the
/// `solution/findings.json` oracle [`extended_key`] writes.
///
/// The original defects are kept verbatim and each accepted adjudication is
/// appended as a [`Defect`]: a deterministic `defect_id` slug, the
/// finding's `file`+`category`, `note` = its description, and a one-line span.
/// Out-of-scope adjudications change nothing, so a run with no acceptances
/// returns the key as-is — the doctrine's "the key stands".
///
/// **Span heuristic (a known approximation).** A Crucible finding anchors to a
/// single reviewer `line`, but a `daedalus-score` defect is a `[line_start,
/// line_end]` span. With only a point anchor we set `line_start == line_end ==
/// line` — the tightest honest span. It is an under-approximation: it does not
/// reconstruct the multi-line region a hand-authored Daedalus defect would
/// cover, so a re-scored finding must hit that exact line to match (not merely
/// fall in a wider seeded band). A later step that carries a region anchor can
/// widen it; until then, exact-line is the floor that cannot silently inflate
/// recall.
///
/// **Severity is deliberately left unset.** The scorer's severity gate ranks
/// only `blocking` > `serious` > `minor`, but Cerberus severities map to
/// `blocking`/`minor`/`info`; an accepted `info` finding would fail to rank and
/// the very finding we accepted would re-score as a false positive. Omitting
/// severity makes the defect match on `file`+`category`+span alone — and mirrors
/// the real severity-less `tests/expected.json` shape.
///
/// **Idempotent by coverage.** An accept is skipped when an existing defect
/// already covers it — same `file` and `category`, with the accept's `line`
/// inside the defect's `[line_start, line_end]` span. This is exactly
/// `daedalus-score`'s own match rule, so a redundant defect — a duplicate accept,
/// a re-export, or an accept that falls inside an original seeded span — is never
/// seeded a second time to inflate the recall denominator. The slug
/// (`defect_id`) stays the appended defect's id.
pub fn extended_expected_key(
    original: &ExpectedKey,
    adjudications: &[Adjudication],
) -> ExpectedKey {
    let mut defects = original.defects.clone();
    for adj in adjudications.iter().filter(|a| a.is_accept()) {
        if defects.iter().any(|d| {
            d.file == adj.file
                && d.category == adj.category
                && d.line_start <= adj.line
                && adj.line <= d.line_end
        }) {
            continue;
        }
        defects.push(Defect {
            id: defect_id(adj),
            file: adj.file.clone(),
            line_start: adj.line,
            line_end: adj.line,
            category: adj.category.clone(),
            severity: None,
            note: adj.description.clone(),
        });
    }
    ExpectedKey { defects }
}

/// A deterministic, file-unique defect id for an accepted finding: a slug of
/// `category` + `file:line`, e.g. `resource-leak-app-auth-py-6`. Deterministic so
/// re-extending the same accept is idempotent ([`extended_expected_key`]), and
/// `file:line`-scoped so two accepts in different places never collide.
fn defect_id(adj: &Adjudication) -> String {
    slugify(&format!("{}-{}-{}", adj.category, adj.file, adj.line))
}

/// Lowercase `s` and collapse every run of non-`[a-z0-9]` characters to a single
/// `-`, trimmed — so `app/auth.py` becomes `app-auth-py`. ASCII-only by design:
/// the inputs are categories and repo paths.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = true; // leading position: suppress separators
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    out
}

/// Render adjudications as an `adjudications.md` in Daedalus's real shape.
///
/// Layout: the title, the doctrine `PREAMBLE`, the
/// `| id | date | task | finding | ruling |` summary table, then one `##` section
/// per adjudication carrying the full, parse-authoritative fields. The output
/// ends with a single trailing newline. Inverse of [`parse_adjudications_md`].
pub fn render_adjudications_md(arena: &str, adjudications: &[Adjudication]) -> String {
    let mut out = String::new();
    out.push_str(&format!("{TITLE_PREFIX}{}\n\n", escape_field(arena)));
    out.push_str(PREAMBLE);
    out.push('\n');
    out.push_str("| id | date | task | finding | ruling |\n");
    out.push_str("|---|---|---|---|---|\n");
    for adj in adjudications {
        // Every free-text cell goes through `cell` (single-line, pipe-safe): a raw
        // newline in `date`/`task` would otherwise split the row and could start a
        // line the parser mistakes for a `## ` section.
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            cell(&adj.id),
            cell(&adj.date),
            cell(&adj.task),
            finding_cell(&adj.description),
            adj.ruling.table_cell(),
        ));
    }
    for adj in adjudications {
        out.push('\n');
        render_section(&mut out, adj);
    }
    out
}

/// Append one adjudication's `##` section to `out`.
///
/// Every free-text field is [`escape_field`]d, not just the claim: a raw newline
/// or `##` in `category`, `file`, `task`, or any other field would otherwise
/// break the bullet line or inject a spurious `##` section, corrupting the
/// artifact's own re-parse. Fields that are controlled enums or integers
/// (verdict, disposition, ruling tag, version, line) carry no hostile content and
/// are rendered raw.
fn render_section(out: &mut String, adj: &Adjudication) {
    out.push_str(&format!(
        "## {}{HEADING_SEP}{} at {}:{} ({})\n\n",
        escape_field(&adj.id),
        escape_field(&adj.category),
        escape_field(&adj.file),
        adj.line,
        adj.ruling.tag(),
    ));
    bullet(out, "Date", &escape_field(&adj.date));
    bullet(out, "Task", &escape_field(&adj.task));
    bullet(out, "Finding id", &escape_field(&adj.finding_id));
    bullet(
        out,
        "Location",
        &format!("{}:{}", escape_field(&adj.file), adj.line),
    );
    bullet(out, "Category", &escape_field(&adj.category));
    bullet(out, "Severity", &escape_field(&adj.severity));
    bullet(out, "Verdict", verdict_label(adj.verdict));
    bullet(out, "Disposition", disposition_label(adj.disposition));
    bullet(out, "Ruling", adj.ruling.tag());
    if let Ruling::Accept { from, to } = &adj.ruling {
        bullet(out, "Version", &format!("{from} → {to}"));
    }
    bullet(out, "Claim", &escape_field(&adj.description));
    bullet(
        out,
        "Conditions",
        &format!(
            "latency_ms={} saw_grader_before_commit={} timestamp={}",
            adj.conditions.latency_ms,
            adj.conditions.saw_grader_before_commit,
            escape_field(&adj.conditions.timestamp),
        ),
    );
}

/// Append a `- **Label:** value` bullet, omitting the trailing space when the
/// value is empty so no rendered line carries trailing whitespace.
fn bullet(out: &mut String, label: &str, value: &str) {
    if value.is_empty() {
        out.push_str(&format!("- **{label}:**\n"));
    } else {
        out.push_str(&format!("- **{label}:** {value}\n"));
    }
}

/// Parse an `adjudications.md` back into its arena and adjudications.
///
/// Reads the title for the arena, then every `##` section for its
/// parse-authoritative bullet fields; the summary table is skipped (it is a
/// regenerated human view). Inverse of [`render_adjudications_md`]:
/// `parse(render(arena, adjs))` recovers `(arena, adjs)` exactly.
pub fn parse_adjudications_md(md: &str) -> Result<ParsedAdjudications, ExportError> {
    let arena = md
        .lines()
        .find_map(|l| l.strip_prefix(TITLE_PREFIX))
        .map(|a| unescape_field(a.trim()))
        .ok_or_else(|| ExportError::parse("missing title line"))?;

    let mut adjudications = Vec::new();
    let mut current: Option<(String, HashMap<String, String>)> = None;
    for line in md.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some((id, fields)) = current.take() {
                adjudications.push(adjudication_from_fields(&id, &fields)?);
            }
            let id = heading
                .split(HEADING_SEP)
                .next()
                .unwrap_or(heading)
                .trim()
                .to_string();
            current = Some((id, HashMap::new()));
        } else if let Some((label, value)) = parse_bullet(line) {
            if let Some((_, fields)) = current.as_mut() {
                fields.insert(label, value);
            }
        }
    }
    if let Some((id, fields)) = current.take() {
        adjudications.push(adjudication_from_fields(&id, &fields)?);
    }

    Ok(ParsedAdjudications {
        arena,
        adjudications,
    })
}

/// Reconstruct one [`Adjudication`] from a section's id and parsed bullet fields.
fn adjudication_from_fields(
    id: &str,
    fields: &HashMap<String, String>,
) -> Result<Adjudication, ExportError> {
    let get = |key: &str| {
        fields
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| ExportError::parse(format!("section {id} is missing field {key:?}")))
    };

    let (file, line) = split_location(get("Location")?, id)?;
    let verdict = parse_verdict(get("Verdict")?)
        .ok_or_else(|| ExportError::parse(format!("section {id} has an unknown verdict")))?;
    let disposition = parse_disposition(get("Disposition")?)
        .ok_or_else(|| ExportError::parse(format!("section {id} has an unknown disposition")))?;
    let ruling = parse_ruling(
        get("Ruling")?,
        fields.get("Version").map(String::as_str),
        id,
    )?;

    Ok(Adjudication {
        id: unescape_field(id),
        date: unescape_field(get("Date")?),
        task: unescape_field(get("Task")?),
        finding_id: unescape_field(get("Finding id")?),
        file,
        line,
        category: unescape_field(get("Category")?),
        severity: unescape_field(get("Severity")?),
        description: unescape_field(get("Claim")?),
        verdict,
        disposition,
        ruling,
        conditions: parse_conditions(get("Conditions")?, id)?,
    })
}

/// Split a `file:line` location, erroring with the section id for context.
///
/// Splits on the *last* `:` — the structural separator before the line number.
/// [`escape_field`] never emits a `:`, so an escaped file (even one containing
/// colons or escaped newlines) keeps the line delimiter unambiguous; the file
/// part is then unescaped back to its raw form.
fn split_location(value: &str, id: &str) -> Result<(String, u32), ExportError> {
    let (file, line) = value.rsplit_once(':').ok_or_else(|| {
        ExportError::parse(format!("section {id} location {value:?} is not file:line"))
    })?;
    let line = line.parse::<u32>().map_err(|_| {
        ExportError::parse(format!("section {id} location line {line:?} is not a u32"))
    })?;
    Ok((unescape_field(file), line))
}

/// Reconstruct a [`Ruling`] from its tag and an optional `Version` field.
fn parse_ruling(tag: &str, version: Option<&str>, id: &str) -> Result<Ruling, ExportError> {
    match tag {
        "ACCEPT" => {
            let version = version.ok_or_else(|| {
                ExportError::parse(format!("section {id} is ACCEPT but has no Version"))
            })?;
            let (from, to) = version.split_once(" → ").ok_or_else(|| {
                ExportError::parse(format!("section {id} version {version:?} is not from → to"))
            })?;
            Ok(Ruling::Accept {
                from: from.trim().parse()?,
                to: to.trim().parse()?,
            })
        }
        "OUT-OF-SCOPE" => Ok(Ruling::OutOfScope),
        other => Err(ExportError::parse(format!(
            "section {id} has an unknown ruling {other:?}"
        ))),
    }
}

/// Parse the `Conditions` field back into the export's [`Conditions`].
///
/// `timestamp` is escaped and rendered last, so it is taken as the entire
/// remainder after `timestamp=` — an escaped timestamp containing spaces still
/// round-trips. The numeric `latency_ms` and the bool never contain whitespace,
/// so the preceding head tokenizes cleanly.
fn parse_conditions(value: &str, id: &str) -> Result<Conditions, ExportError> {
    let mut conditions = Conditions::default();
    let head = match value.split_once("timestamp=") {
        Some((head, ts)) => {
            conditions.timestamp = unescape_field(ts);
            head
        }
        None => value,
    };
    for tok in head.split_whitespace() {
        if let Some(v) = tok.strip_prefix("latency_ms=") {
            conditions.latency_ms = v.parse().map_err(|_| {
                ExportError::parse(format!("section {id} latency_ms {v:?} is not a u64"))
            })?;
        } else if let Some(v) = tok.strip_prefix("saw_grader_before_commit=") {
            conditions.saw_grader_before_commit = v.parse().map_err(|_| {
                ExportError::parse(format!(
                    "section {id} saw_grader_before_commit {v:?} is not a bool"
                ))
            })?;
        }
    }
    Ok(conditions)
}

/// Parse a `- **Label:** value` bullet into `(label, value)`; `None` for any
/// other line. An empty value (`- **Label:**`) yields an empty string.
fn parse_bullet(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("- **")?;
    let idx = rest.find(":**")?;
    let label = rest[..idx].to_string();
    let after = &rest[idx + ":**".len()..];
    let value = after.strip_prefix(' ').unwrap_or(after).to_string();
    Some((label, value))
}

/// A summary-table cell: a value's first line, trimmed and pipe-escaped. The
/// table is a regenerated human view ignored on parse, so collapsing to the first
/// line loses nothing recoverable — but it must never carry a raw newline (which
/// would split the row and could start a line the parser reads as a `## ` section)
/// or a raw `|` (which would break the columns).
fn cell(value: &str) -> String {
    value
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .replace('|', "\\|")
}

/// The summary table's `finding` cell: [`cell`] plus truncation to
/// [`FINDING_CELL_WIDTH`]. A lossy human view — the section's `Claim` field is
/// authoritative.
fn finding_cell(description: &str) -> String {
    let escaped = cell(description);
    if escaped.chars().count() <= FINDING_CELL_WIDTH {
        return escaped;
    }
    let head: String = escaped
        .chars()
        .take(FINDING_CELL_WIDTH.saturating_sub(1).max(1))
        .collect();
    format!("{head}…")
}

/// `keep` / `nit` / `wrong` / `noise` — the [`Verdict`] wire form, reused so the
/// Markdown field reads the same vocabulary as the JSON.
fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Keep => "keep",
        Verdict::Nit => "nit",
        Verdict::Wrong => "wrong",
        Verdict::Noise => "noise",
    }
}

/// Inverse of [`verdict_label`].
fn parse_verdict(s: &str) -> Option<Verdict> {
    match s {
        "keep" => Some(Verdict::Keep),
        "nit" => Some(Verdict::Nit),
        "wrong" => Some(Verdict::Wrong),
        "noise" => Some(Verdict::Noise),
        _ => None,
    }
}

/// `in-scope` / `out-of-contract` — the scope disposition as readable prose.
fn disposition_label(disposition: Disposition) -> &'static str {
    if disposition.in_scope {
        "in-scope"
    } else {
        "out-of-contract"
    }
}

/// Inverse of [`disposition_label`].
fn parse_disposition(s: &str) -> Option<Disposition> {
    match s {
        "in-scope" => Some(Disposition { in_scope: true }),
        "out-of-contract" => Some(Disposition { in_scope: false }),
        _ => None,
    }
}

/// Escape a free-text field so it survives a single-line Markdown bullet: a
/// reversible mapping of backslash and the three ASCII whitespace controls.
fn escape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Inverse of [`escape_field`]. A stray, unrecognized escape is passed through
/// verbatim so a hand-edited file never silently loses a backslash.
fn unescape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grade::GradeResult;
    use crate::judgment::{apply_label, build_queue, LabelConditions};

    fn cand(
        file: &str,
        line: u32,
        category: &str,
        source_id: &str,
        description: &str,
    ) -> KeyFinding {
        KeyFinding {
            file: file.to_string(),
            line,
            category: category.to_string(),
            severity: "blocking".to_string(),
            description: description.to_string(),
            source_id: Some(source_id.to_string()),
        }
    }

    /// Build a labeled queue from a set of disputed candidates and the decisions
    /// taken on them, through the real public minting path (no mocks).
    fn labeled_queue(disputes: Vec<(KeyFinding, Verdict, bool, LabelConditions)>) -> JudgmentQueue {
        let grade = GradeResult {
            matched: Vec::new(),
            disputed: disputes.iter().map(|(c, _, _, _)| c.clone()).collect(),
            missed: Vec::new(),
        };
        let mut queue = build_queue(&grade);
        let items = queue.items.clone();
        for (item, (_, verdict, in_scope, conditions)) in items.iter().zip(disputes) {
            queue.labels.push(apply_label(
                item,
                verdict,
                Disposition { in_scope },
                &conditions,
            ));
        }
        queue
    }

    fn ctx() -> ExportContext {
        ExportContext {
            arena: "pr-review-v0".to_string(),
            task: "py-file-cache".to_string(),
            date: "2026-06-29".to_string(),
            base_version: ArenaVersion {
                major: 0,
                minor: 2,
                patch: 0,
            },
        }
    }

    // ---- ArenaVersion -----------------------------------------------------

    #[test]
    fn version_parses_displays_and_bumps() {
        let v: ArenaVersion = "0.2.1".parse().unwrap();
        assert_eq!(v.to_string(), "0.2.1");
        assert_eq!(
            v.bump_minor().unwrap().to_string(),
            "0.3.0",
            "patch resets on a minor bump"
        );
    }

    #[test]
    fn version_rejects_malformed_with_a_version_specific_error() {
        for bad in ["0.2", "0.2.0.1", "a.b.c"] {
            let err = bad.parse::<ArenaVersion>().unwrap_err();
            // A bad version must NOT masquerade as a malformed adjudications.md —
            // it surfaces as a version error naming the offending value.
            assert!(
                matches!(&err, ExportError::Version { value, .. } if value == bad),
                "expected a Version error for {bad:?}, got {err:?}"
            );
            let shown = err.to_string();
            assert!(
                shown.contains("invalid arena version") && !shown.contains("adjudications.md"),
                "version error must name the version, not adjudications.md: {shown}"
            );
        }
    }

    #[test]
    fn bump_minor_errors_on_u32_overflow() {
        let ceiling = ArenaVersion {
            major: 0,
            minor: u32::MAX,
            patch: 0,
        };
        let err = ceiling.bump_minor().unwrap_err();
        assert!(
            matches!(err, ExportError::VersionOverflow { version } if version == ceiling),
            "a minor bump past u32::MAX is caught, not wrapped: {err:?}"
        );
    }

    // ---- decision rule ----------------------------------------------------

    #[test]
    fn only_keep_and_in_scope_is_accept() {
        // ACCEPT requires both axes; every other pair leaves the key unchanged.
        assert!(is_accept(Verdict::Keep, Disposition { in_scope: true }));
        assert!(!is_accept(Verdict::Keep, Disposition { in_scope: false }));
        assert!(!is_accept(Verdict::Nit, Disposition { in_scope: true }));
        assert!(!is_accept(Verdict::Wrong, Disposition { in_scope: true }));
        assert!(!is_accept(Verdict::Noise, Disposition { in_scope: true }));
        assert!(!is_accept(Verdict::Noise, Disposition { in_scope: false }));
    }

    // ---- adjudications_from_queue + version sequencing --------------------

    #[test]
    fn accepts_walk_the_version_forward_one_minor_each() {
        // Two accepts around one out-of-scope: the version walks 0.2.0 -> 0.3.0
        // -> 0.4.0, the out-of-scope entry does not bump, and ids run in order.
        let queue = labeled_queue(vec![
            (
                cand("a.rs", 5, "concurrency", "F1", "race"),
                Verdict::Keep,
                true,
                LabelConditions::default(),
            ),
            (
                cand("b.rs", 9, "portability", "F2", "windows only"),
                Verdict::Keep,
                false,
                LabelConditions::default(),
            ),
            (
                cand("c.rs", 12, "security", "F3", "second real defect"),
                Verdict::Keep,
                true,
                LabelConditions::default(),
            ),
        ]);
        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        assert_eq!(adjs.len(), 3);
        assert_eq!(adjs[0].id, "ADJ-1");
        assert_eq!(
            adjs[0].ruling,
            Ruling::Accept {
                from: ArenaVersion {
                    major: 0,
                    minor: 2,
                    patch: 0
                },
                to: ArenaVersion {
                    major: 0,
                    minor: 3,
                    patch: 0
                },
            }
        );
        assert_eq!(adjs[1].id, "ADJ-2");
        assert_eq!(
            adjs[1].ruling,
            Ruling::OutOfScope,
            "out-of-scope does not bump"
        );
        assert_eq!(
            adjs[2].ruling,
            Ruling::Accept {
                from: ArenaVersion {
                    major: 0,
                    minor: 3,
                    patch: 0
                },
                to: ArenaVersion {
                    major: 0,
                    minor: 4,
                    patch: 0
                },
            },
            "the second accept bumps from the post-first-accept version"
        );
    }

    #[test]
    fn adjudication_copies_the_disputed_finding_and_conditions() {
        let queue = labeled_queue(vec![(
            cand("cache.py", 23, "concurrency", "F3", "tmp write race"),
            Verdict::Keep,
            true,
            LabelConditions {
                latency_ms: 90_000,
                saw_grader_before_commit: false,
                timestamp: "2026-06-29T18:12:00Z".to_string(),
            },
        )]);
        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        let adj = &adjs[0];
        assert_eq!(adj.finding_id, "F3");
        assert_eq!(adj.file, "cache.py");
        assert_eq!(adj.line, 23);
        assert_eq!(adj.category, "concurrency");
        assert_eq!(adj.description, "tmp write race");
        assert_eq!(adj.conditions.latency_ms, 90_000);
        assert_eq!(adj.conditions.timestamp, "2026-06-29T18:12:00Z");
        assert!(adj.is_accept());
    }

    #[test]
    fn a_dangling_label_is_an_error() {
        // A label whose finding id names no queue item cannot recover its claim.
        let mut queue = labeled_queue(vec![(
            cand("a.rs", 5, "security", "F1", "d"),
            Verdict::Keep,
            true,
            LabelConditions::default(),
        )]);
        queue.labels[0].finding_id = "ghost".to_string();
        let err = adjudications_from_queue(&queue, &ctx()).unwrap_err();
        assert!(matches!(err, ExportError::DanglingLabel { finding_id } if finding_id == "ghost"));
    }

    // ---- label reconciliation (append-only corrections) -------------------

    /// Append a second label for an existing item's finding — an append-only
    /// correction the export must reconcile.
    fn correct(queue: &mut JudgmentQueue, verdict: Verdict, in_scope: bool, timestamp: &str) {
        let item = queue.items[0].clone();
        queue.labels.push(apply_label(
            &item,
            verdict,
            Disposition { in_scope },
            &LabelConditions {
                latency_ms: 0,
                saw_grader_before_commit: false,
                timestamp: timestamp.to_string(),
            },
        ));
    }

    #[test]
    fn a_retracted_accept_does_not_extend_the_key_or_bump_the_version() {
        // F3 was ACCEPTed, then re-ruled noise with a later timestamp. Only the
        // correction is in force: one OUT-OF-SCOPE adjudication, no key change.
        let mut queue = labeled_queue(vec![(
            cand("cache.py", 23, "concurrency", "F3", "tmp write race"),
            Verdict::Keep,
            true,
            LabelConditions {
                latency_ms: 0,
                saw_grader_before_commit: false,
                timestamp: "2026-06-29T10:00:00Z".to_string(),
            },
        )]);
        correct(&mut queue, Verdict::Noise, false, "2026-06-29T12:00:00Z");

        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        assert_eq!(adjs.len(), 1, "the two labels collapse to one adjudication");
        assert_eq!(adjs[0].verdict, Verdict::Noise, "the correction wins");
        assert!(
            matches!(adjs[0].ruling, Ruling::OutOfScope),
            "a retracted accept does not extend the key"
        );
        // The key stands in BOTH outputs — no oracle row, no scorer defect.
        let oracle = extended_key(
            &AnswerKey {
                findings: Vec::new(),
            },
            &adjs,
        );
        assert!(oracle.findings.is_empty(), "no solution/findings.json row");
        let scorer = extended_expected_key(&ExpectedKey::default(), &adjs);
        assert!(scorer.defects.is_empty(), "no tests/expected.json defect");
    }

    #[test]
    fn a_duplicated_accept_is_counted_once() {
        // The same ACCEPT committed twice collapses to one adjudication and one
        // minor bump, and seeds the scorer key exactly once.
        let mut queue = labeled_queue(vec![(
            cand("cache.py", 23, "concurrency", "F3", "tmp write race"),
            Verdict::Keep,
            true,
            LabelConditions {
                latency_ms: 0,
                saw_grader_before_commit: false,
                timestamp: "2026-06-29T10:00:00Z".to_string(),
            },
        )]);
        correct(&mut queue, Verdict::Keep, true, "2026-06-29T11:00:00Z");

        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        assert_eq!(
            adjs.len(),
            1,
            "duplicate accepts collapse to one adjudication"
        );
        match adjs[0].ruling {
            Ruling::Accept { from, to } => {
                assert_eq!(from.to_string(), "0.2.0");
                assert_eq!(to.to_string(), "0.3.0", "only one minor bump, not two");
            }
            Ruling::OutOfScope => panic!("the surviving accept must still ACCEPT"),
        }
        let scorer = extended_expected_key(&ExpectedKey::default(), &adjs);
        assert_eq!(scorer.defects.len(), 1, "no double-seeded defect");
    }

    // ---- extended_key -----------------------------------------------------

    #[test]
    fn extended_key_appends_only_accepts_and_drops_source_id() {
        let original = AnswerKey {
            findings: vec![KeyFinding {
                file: "cache.py".to_string(),
                line: 8,
                category: "security".to_string(),
                severity: String::new(),
                description: "path traversal".to_string(),
                source_id: None,
            }],
        };
        let queue = labeled_queue(vec![
            (
                cand("cache.py", 23, "concurrency", "F3", "tmp write race"),
                Verdict::Keep,
                true,
                LabelConditions::default(),
            ),
            (
                cand("cache.py", 26, "portability", "F2", "windows rename"),
                Verdict::Keep,
                false, // out of scope -> not added
                LabelConditions::default(),
            ),
        ]);
        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        let extended = extended_key(&original, &adjs);
        assert_eq!(extended.findings.len(), 2, "one original + one accept");
        let added = &extended.findings[1];
        assert_eq!(added.category, "concurrency");
        assert_eq!(added.line, 23);
        assert!(added.source_id.is_none(), "key rows carry no source id");
    }

    #[test]
    fn extended_key_with_no_accepts_is_unchanged() {
        let original = AnswerKey {
            findings: vec![KeyFinding {
                file: "a.rs".to_string(),
                line: 1,
                category: "security".to_string(),
                severity: String::new(),
                description: "d".to_string(),
                source_id: None,
            }],
        };
        let queue = labeled_queue(vec![(
            cand("b.rs", 9, "portability", "F2", "out of scope"),
            Verdict::Keep,
            false,
            LabelConditions::default(),
        )]);
        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        assert_eq!(extended_key(&original, &adjs), original, "the key stands");
    }

    #[test]
    fn extended_key_is_idempotent_by_location_and_category() {
        // An accept whose (file, line, category) a row already covers is skipped:
        // re-exporting never grows a duplicate oracle row.
        let original = AnswerKey {
            findings: vec![KeyFinding {
                file: "cache.py".to_string(),
                line: 23,
                category: "concurrency".to_string(),
                severity: String::new(),
                description: "already in the key".to_string(),
                source_id: None,
            }],
        };
        let queue = labeled_queue(vec![(
            cand("cache.py", 23, "concurrency", "F3", "tmp write race"),
            Verdict::Keep,
            true,
            LabelConditions::default(),
        )]);
        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        assert_eq!(
            extended_key(&original, &adjs),
            original,
            "an accept already covered by a key row is not appended"
        );
        // Re-extending the once-extended key is a no-op too.
        let once = extended_key(
            &AnswerKey {
                findings: Vec::new(),
            },
            &adjs,
        );
        assert_eq!(once.findings.len(), 1);
        assert_eq!(
            extended_key(&once, &adjs),
            once,
            "a second pass adds nothing"
        );
    }

    // ---- extended_expected_key (tests/expected.json) ----------------------

    #[test]
    fn extended_expected_key_appends_accept_defects_with_span_heuristic() {
        let original = ExpectedKey {
            defects: vec![Defect {
                id: "seeded".to_string(),
                file: "cache.py".to_string(),
                line_start: 8,
                line_end: 12,
                category: "security".to_string(),
                severity: None,
                note: "path traversal".to_string(),
            }],
        };
        let queue = labeled_queue(vec![
            (
                cand("cache.py", 23, "concurrency", "F3", "tmp write race"),
                Verdict::Keep,
                true,
                LabelConditions::default(),
            ),
            (
                cand("cache.py", 26, "portability", "F2", "windows rename"),
                Verdict::Keep,
                false, // out of scope -> not seeded
                LabelConditions::default(),
            ),
        ]);
        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        let extended = extended_expected_key(&original, &adjs);

        assert_eq!(extended.defects.len(), 2, "one seeded + one accept");
        assert_eq!(
            extended.defects[0], original.defects[0],
            "seeded defect verbatim"
        );
        let added = &extended.defects[1];
        assert_eq!(
            (added.line_start, added.line_end),
            (23, 23),
            "the point anchor collapses to a one-line span"
        );
        assert_eq!(added.category, "concurrency");
        assert_eq!(added.note, "tmp write race", "note carries the description");
        assert!(
            added.severity.is_none(),
            "the written defect omits severity"
        );
        assert_eq!(added.id, "concurrency-cache-py-23", "deterministic slug id");
    }

    #[test]
    fn extended_expected_key_with_no_accepts_is_unchanged() {
        let original = ExpectedKey {
            defects: vec![Defect {
                id: "seeded".to_string(),
                file: "a.rs".to_string(),
                line_start: 1,
                line_end: 1,
                category: "security".to_string(),
                severity: None,
                note: "d".to_string(),
            }],
        };
        let queue = labeled_queue(vec![(
            cand("b.rs", 9, "portability", "F2", "out of scope"),
            Verdict::Keep,
            false,
            LabelConditions::default(),
        )]);
        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        assert_eq!(
            extended_expected_key(&original, &adjs),
            original,
            "the key stands"
        );
    }

    #[test]
    fn extended_expected_key_is_idempotent_by_id() {
        let queue = labeled_queue(vec![(
            cand("cache.py", 23, "concurrency", "F3", "tmp write race"),
            Verdict::Keep,
            true,
            LabelConditions::default(),
        )]);
        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        let once = extended_expected_key(&ExpectedKey::default(), &adjs);
        assert_eq!(once.defects.len(), 1);
        let twice = extended_expected_key(&once, &adjs);
        assert_eq!(
            twice, once,
            "re-extending the same accept is a no-op (deterministic id dedup)"
        );
    }

    #[test]
    fn extended_expected_key_skips_an_accept_inside_an_existing_span() {
        // An accept whose line falls inside a seeded defect's span (same file +
        // category) is already credited by daedalus-score, so it is not re-seeded
        // — the (file, line, category) coverage check, generalized to spans.
        let original = ExpectedKey {
            defects: vec![Defect {
                id: "seeded-span".to_string(),
                file: "cache.py".to_string(),
                line_start: 20,
                line_end: 30,
                category: "concurrency".to_string(),
                severity: None,
                note: "a wide seeded span".to_string(),
            }],
        };
        let queue = labeled_queue(vec![(
            cand("cache.py", 23, "concurrency", "F3", "inside the span"),
            Verdict::Keep,
            true,
            LabelConditions::default(),
        )]);
        let adjs = adjudications_from_queue(&queue, &ctx()).unwrap();
        assert_eq!(
            extended_expected_key(&original, &adjs),
            original,
            "an accept inside an existing same-category span is not re-seeded"
        );
    }

    #[test]
    fn slugify_lowercases_collapses_and_trims() {
        assert_eq!(
            slugify("resource-leak-app/auth.py-6"),
            "resource-leak-app-auth-py-6"
        );
        assert_eq!(slugify("Runtime Crash"), "runtime-crash");
        assert_eq!(slugify("  /weird//path__name  "), "weird-path-name");
    }

    // ---- escaping ---------------------------------------------------------

    #[test]
    fn escape_round_trips_control_chars_and_backslashes() {
        for raw in [
            "plain",
            "title\n\nbody with a tab\tand more",
            "carriage\r\nreturn",
            "a literal backslash-n: \\n stays distinct from a newline",
            "trailing backslash\\",
        ] {
            assert_eq!(
                unescape_field(&escape_field(raw)),
                raw,
                "round-trip {raw:?}"
            );
            assert!(
                !escape_field(raw).contains('\n'),
                "escaped field is single-line"
            );
        }
    }

    // ---- render <-> parse round-trip --------------------------------------

    #[test]
    fn render_then_parse_is_lossless() {
        // A queue exercising every branch: an accept (with a multi-line claim and
        // full conditions) and two out-of-scope rulings (one correct-but-foreign,
        // one confirmed noise with an empty timestamp).
        let queue = labeled_queue(vec![
            (
                cand(
                    "cache.py",
                    23,
                    "concurrency",
                    "F3",
                    "Concurrent set() writers race on the temp file.\n\nTwo writers for one key interleave writes to the same deterministic .tmp; the rename can publish corrupted JSON.",
                ),
                Verdict::Keep,
                true,
                LabelConditions {
                    latency_ms: 90_000,
                    saw_grader_before_commit: false,
                    timestamp: "2026-06-29T18:12:00Z".to_string(),
                },
            ),
            (
                cand("cache.py", 26, "portability", "F1", "os.rename raises on Windows when the destination exists."),
                Verdict::Keep,
                false,
                LabelConditions {
                    latency_ms: 45_000,
                    saw_grader_before_commit: false,
                    timestamp: "2026-06-29T18:15:00Z".to_string(),
                },
            ),
            (
                cand("app.py", 5, "style", "F2", "Prefer f-strings here."),
                Verdict::Noise,
                true,
                LabelConditions::default(), // empty timestamp
            ),
        ]);
        let context = ctx();
        let adjs = adjudications_from_queue(&queue, &context).unwrap();
        let md = render_adjudications_md(&context.arena, &adjs);

        let parsed = parse_adjudications_md(&md).unwrap();
        assert_eq!(parsed.arena, context.arena);
        assert_eq!(
            parsed.adjudications, adjs,
            "every field survives the round-trip"
        );

        // And render is a fixpoint: re-rendering the parse yields identical bytes.
        assert_eq!(
            render_adjudications_md(&parsed.arena, &parsed.adjudications),
            md
        );
    }

    #[test]
    fn render_then_parse_survives_hostile_field_content() {
        // Newlines, an injected `## ` heading, pipes, colons, tabs, and backslashes
        // in EVERY free-text field must survive the artifact's own round-trip —
        // none may break a bullet, split the summary table, or inject a spurious
        // section. (The id is Crucible-generated, so it stays a clean ADJ-N.)
        let hostile =
            "line one\n## ADJ-99 — injected at evil.rs:1 (ACCEPT)\nlit\\nbackslash\ttab|pipe:colon";
        let adj = Adjudication {
            id: "ADJ-1".to_string(),
            date: hostile.to_string(),
            task: hostile.to_string(),
            finding_id: hostile.to_string(),
            file: hostile.to_string(),
            line: 7,
            category: hostile.to_string(),
            severity: hostile.to_string(),
            description: hostile.to_string(),
            verdict: Verdict::Keep,
            disposition: Disposition { in_scope: true },
            ruling: Ruling::Accept {
                from: ArenaVersion {
                    major: 0,
                    minor: 2,
                    patch: 0,
                },
                to: ArenaVersion {
                    major: 0,
                    minor: 3,
                    patch: 0,
                },
            },
            conditions: Conditions {
                latency_ms: 90_000,
                saw_grader_before_commit: true,
                timestamp: "ts with a space and ## hash".to_string(),
            },
        };
        // A hostile arena id too, to exercise the title round-trip.
        let arena = "arena\n## not-a-section";
        let md = render_adjudications_md(arena, std::slice::from_ref(&adj));

        let parsed = parse_adjudications_md(&md).expect("hostile content still parses");
        assert_eq!(
            parsed.arena, arena,
            "the title round-trips the hostile arena"
        );
        assert_eq!(
            parsed.adjudications.len(),
            1,
            "the injected `## ` content created no spurious section:\n{md}"
        );
        assert_eq!(
            parsed.adjudications[0], adj,
            "every hostile field round-trips intact"
        );
    }

    #[test]
    fn rendered_artifact_matches_the_real_daedalus_shape() {
        let queue = labeled_queue(vec![(
            cand("cache.py", 23, "concurrency", "F3", "tmp write race"),
            Verdict::Keep,
            true,
            LabelConditions::default(),
        )]);
        let context = ctx();
        let adjs = adjudications_from_queue(&queue, &context).unwrap();
        let md = render_adjudications_md(&context.arena, &adjs);

        assert!(md.starts_with("# Answer-key adjudications — pr-review-v0\n"));
        assert!(md.contains("| id | date | task | finding | ruling |"));
        assert!(
            md.contains("**ACCEPT** → key extended, arena 0.2.0 → 0.3.0"),
            "the ACCEPT cell matches Daedalus's exact wording:\n{md}"
        );
        assert!(md.contains("## ADJ-1 — concurrency at cache.py:23 (ACCEPT)"));
        assert!(md.ends_with('\n'));
    }

    #[test]
    fn parse_rejects_a_file_without_a_title() {
        let err = parse_adjudications_md("no title here\n").unwrap_err();
        assert!(matches!(err, ExportError::Parse { .. }));
    }
}
