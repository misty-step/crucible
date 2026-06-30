//! Adapter: Cerberus review findings -> Daedalus answer-key rows.
//!
//! Crucible scores a Cerberus review against a Daedalus key, but the two speak
//! different vocabularies. This module is the one-way bridge: it loads a
//! [`ReviewArtifact`] from disk and projects each [`Finding`] into the
//! [`KeyFinding`] shape Daedalus uses (`{ file, line, category, severity,
//! description }`), so a review and a key can be compared on equal terms.
//!
//! The projection is **total and order-preserving** — every finding yields
//! exactly one key row, even one with no usable anchor — so a downstream matcher
//! sees the whole review, not a silently filtered subset. Three deliberate
//! mapping choices:
//!
//! - **Location** (`file`/`line`) comes from the finding's most specific
//!   location anchor. An `Inline` or `Change` anchor pins a line in the diff and
//!   is preferred; a path-only `File` anchor is the fallback. The chosen
//!   anchor's `line` is used, falling back to `start_line`. A finding with no
//!   path-bearing anchor maps to an empty `file` and line `0` — the "unanchored"
//!   sentinel a matcher can detect and route, rather than the adapter dropping
//!   the row.
//! - **Severity** collapses Cerberus's four levels onto Daedalus's vocabulary:
//!   `Critical` and `Major` are both merge-blockers, so both map to `blocking`;
//!   `Minor` -> `minor`; `Info` -> `info`.
//! - **Description** joins the finding's one-line `title` and its fuller
//!   `description`, so the key row keeps both the headline and the rationale.

use std::path::Path;

use crate::artifact::{Anchor, AnchorKind, Finding, ReviewArtifact, Severity};
use crate::error::Result;
use crate::key::KeyFinding;

/// Load a Cerberus artifact from `path` and return its findings.
///
/// A thin file-loading entry point over [`ReviewArtifact::from_path`]: it parses
/// the artifact and hands back its `findings`, discarding the rest of the
/// envelope the eval does not consume. Errors carry the offending path (see
/// [`crate::Error`]).
pub fn findings_from_artifact(path: impl AsRef<Path>) -> Result<Vec<Finding>> {
    Ok(ReviewArtifact::from_path(path)?.findings)
}

/// Project Cerberus findings into Daedalus answer-key rows.
///
/// Total and order-preserving: every finding yields exactly one [`KeyFinding`],
/// in input order. See the module docs for the location, severity, and
/// description mapping rules.
pub fn to_key_findings(findings: &[Finding]) -> Vec<KeyFinding> {
    findings.iter().map(to_key_finding).collect()
}

/// Project one finding into one key row.
fn to_key_finding(finding: &Finding) -> KeyFinding {
    let (file, line) = location(finding);
    KeyFinding {
        file,
        line,
        category: finding.category.clone(),
        severity: map_severity(finding.severity).to_string(),
        description: combined_description(finding),
    }
}

/// Map a Cerberus severity onto the Daedalus severity vocabulary.
///
/// Daedalus keys use a coarser vocabulary than Cerberus's four levels. Both
/// `Critical` and `Major` are merge-blocking, so both collapse to `blocking`;
/// `Minor` and `Info` keep their names.
fn map_severity(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical | Severity::Major => "blocking",
        Severity::Minor => "minor",
        Severity::Info => "info",
    }
}

/// Resolve a finding's `file`/`line` from its best location anchor.
///
/// Returns `("", 0)` when no anchor carries a path — the "unanchored" sentinel
/// (`file` is empty, `line` is `0`). Otherwise the chosen anchor's path and its
/// `line` (or `start_line`) are used.
fn location(finding: &Finding) -> (String, u32) {
    match best_anchor(&finding.anchors) {
        Some(a) => (
            a.path.clone().unwrap_or_default(),
            a.line.or(a.start_line).unwrap_or(0),
        ),
        None => (String::new(), 0),
    }
}

/// Pick the most location-rich anchor: a path-bearing `Inline`/`Change` anchor
/// if present (these pin a line in the diff), else any path-bearing anchor, else
/// `None`. Path-less anchors (e.g. a bare `Run` anchor) are never chosen.
fn best_anchor(anchors: &[Anchor]) -> Option<&Anchor> {
    anchors
        .iter()
        .find(|a| a.path.is_some() && matches!(a.kind, AnchorKind::Inline | AnchorKind::Change))
        .or_else(|| anchors.iter().find(|a| a.path.is_some()))
}

/// Join a finding's `title` and `description` into one key-row description,
/// skipping whichever is blank so the result never carries a stray separator.
fn combined_description(finding: &Finding) -> String {
    let title = finding.title.trim();
    let body = finding.description.trim();
    match (title.is_empty(), body.is_empty()) {
        (false, false) => format!("{title}\n\n{body}"),
        (false, true) => title.to_string(),
        (true, false) => body.to_string(),
        (true, true) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `Finding` with the fields the adapter reads; the rest are filler.
    fn finding(
        severity: Severity,
        category: &str,
        title: &str,
        description: &str,
        anchors: Vec<Anchor>,
    ) -> Finding {
        Finding {
            id: "F1".to_string(),
            severity,
            category: category.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            evidence: "e".to_string(),
            confidence: 0.5,
            anchors,
            citations: Vec::new(),
            suggested_fixes: Vec::new(),
        }
    }

    fn anchor(
        kind: AnchorKind,
        path: Option<&str>,
        line: Option<u32>,
        start_line: Option<u32>,
    ) -> Anchor {
        Anchor {
            kind,
            path: path.map(String::from),
            line,
            start_line,
            end_line: None,
        }
    }

    #[test]
    fn severity_maps_to_daedalus_vocabulary() {
        assert_eq!(map_severity(Severity::Critical), "blocking");
        assert_eq!(map_severity(Severity::Major), "blocking");
        assert_eq!(map_severity(Severity::Minor), "minor");
        assert_eq!(map_severity(Severity::Info), "info");
    }

    #[test]
    fn inline_anchor_supplies_file_and_line() {
        let f = finding(
            Severity::Major,
            "correctness",
            "t",
            "d",
            vec![anchor(
                AnchorKind::Inline,
                Some("src/x.rs"),
                Some(42),
                Some(40),
            )],
        );
        let k = to_key_finding(&f);
        assert_eq!(k.file, "src/x.rs");
        assert_eq!(k.line, 42);
        assert_eq!(k.severity, "blocking");
    }

    #[test]
    fn inline_anchor_preferred_over_earlier_file_anchor() {
        // File anchor appears first, inline second: the line-bearing inline wins.
        let f = finding(
            Severity::Minor,
            "c",
            "t",
            "d",
            vec![
                anchor(AnchorKind::File, Some("file-only.rs"), None, None),
                anchor(AnchorKind::Inline, Some("inline.rs"), Some(7), None),
            ],
        );
        let k = to_key_finding(&f);
        assert_eq!(k.file, "inline.rs");
        assert_eq!(k.line, 7);
    }

    #[test]
    fn change_anchor_is_a_preferred_kind_and_falls_back_to_start_line() {
        let f = finding(
            Severity::Minor,
            "c",
            "t",
            "d",
            vec![anchor(
                AnchorKind::Change,
                Some("changed.rs"),
                None,
                Some(15),
            )],
        );
        let k = to_key_finding(&f);
        assert_eq!(k.file, "changed.rs");
        assert_eq!(k.line, 15, "falls back to start_line when line is absent");
    }

    #[test]
    fn file_anchor_is_fallback_when_no_inline_or_change() {
        // Only a path-only File anchor: path is used, line is the 0 sentinel.
        let f = finding(
            Severity::Info,
            "c",
            "t",
            "d",
            vec![anchor(AnchorKind::File, Some("only.rs"), None, None)],
        );
        let k = to_key_finding(&f);
        assert_eq!(k.file, "only.rs");
        assert_eq!(k.line, 0);
    }

    #[test]
    fn unanchored_finding_maps_to_empty_location_sentinel() {
        let f = finding(Severity::Info, "c", "t", "d", Vec::new());
        let k = to_key_finding(&f);
        assert_eq!(k.file, "");
        assert_eq!(k.line, 0);
    }

    #[test]
    fn pathless_anchor_is_never_chosen() {
        // A bare Run anchor carries no path; the result is the sentinel.
        let f = finding(
            Severity::Info,
            "c",
            "t",
            "d",
            vec![anchor(AnchorKind::Run, None, None, None)],
        );
        let k = to_key_finding(&f);
        assert_eq!(k.file, "");
        assert_eq!(k.line, 0);
    }

    #[test]
    fn line_is_preferred_over_start_line() {
        let f = finding(
            Severity::Minor,
            "c",
            "t",
            "d",
            vec![anchor(AnchorKind::Inline, Some("x.rs"), Some(99), Some(50))],
        );
        assert_eq!(to_key_finding(&f).line, 99);
    }

    #[test]
    fn category_passes_through() {
        let f = finding(Severity::Minor, "runtime-crash", "t", "d", Vec::new());
        assert_eq!(to_key_finding(&f).category, "runtime-crash");
    }

    #[test]
    fn description_joins_title_and_body() {
        let f = finding(
            Severity::Minor,
            "c",
            "Headline",
            "Full rationale.",
            Vec::new(),
        );
        let d = to_key_finding(&f).description;
        assert!(d.contains("Headline"), "missing title in {d:?}");
        assert!(d.contains("Full rationale."), "missing body in {d:?}");
    }

    #[test]
    fn description_skips_blank_parts() {
        let only_title = finding(Severity::Minor, "c", "Headline", "   ", Vec::new());
        assert_eq!(to_key_finding(&only_title).description, "Headline");

        let only_body = finding(Severity::Minor, "c", "", "Body", Vec::new());
        assert_eq!(to_key_finding(&only_body).description, "Body");

        let neither = finding(Severity::Minor, "c", "", "", Vec::new());
        assert_eq!(to_key_finding(&neither).description, "");
    }

    #[test]
    fn to_key_findings_preserves_count_and_order() {
        let findings = vec![
            finding(
                Severity::Critical,
                "a",
                "t1",
                "d1",
                vec![anchor(AnchorKind::Inline, Some("a.rs"), Some(1), None)],
            ),
            finding(
                Severity::Info,
                "b",
                "t2",
                "d2",
                vec![anchor(AnchorKind::Inline, Some("b.rs"), Some(2), None)],
            ),
        ];
        let keys = to_key_findings(&findings);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].file, "a.rs");
        assert_eq!(keys[0].severity, "blocking");
        assert_eq!(keys[1].file, "b.rs");
        assert_eq!(keys[1].severity, "info");
    }

    #[test]
    fn to_key_findings_is_empty_for_no_findings() {
        assert!(to_key_findings(&[]).is_empty());
    }
}
