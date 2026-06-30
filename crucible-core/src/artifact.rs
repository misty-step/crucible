//! Cerberus review-artifact types, mirrored for deserialization.
//!
//! Source of truth: `cerberus/src/schema.rs` (`ReviewArtifact`, `Finding`,
//! `Severity`, `Anchor`). Crucible models only the surface its eval consumes —
//! `schema_version` and `findings` — and lets serde discard the rest of the
//! Cerberus envelope (verdict, summary, receipts, run info, …) so a real
//! artifact deserializes unchanged.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// A Cerberus code-review artifact: the output of one review run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewArtifact {
    /// Schema identifier, e.g. `cerberus.review_artifact.v1`.
    pub schema_version: String,
    /// The findings the reviewer reported. Absent in some artifacts; defaults
    /// to empty.
    #[serde(default)]
    pub findings: Vec<Finding>,
}

impl ReviewArtifact {
    /// Deserialize an artifact from a JSON string.
    pub fn from_json_str(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Load and deserialize an artifact from a JSON file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| Error::read(path, e))?;
        serde_json::from_slice(&bytes).map_err(|e| Error::parse(path, e))
    }
}

/// One reviewer finding. Mirrors Cerberus `Finding`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable per-artifact identifier, e.g. `F1`.
    pub id: String,
    /// Reviewer-assigned severity.
    pub severity: Severity,
    /// Free-form category, e.g. `security`.
    pub category: String,
    /// One-line headline.
    pub title: String,
    /// Full explanation of the finding.
    pub description: String,
    /// The concrete evidence the reviewer cites.
    pub evidence: String,
    /// Reviewer confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Where the finding points in the change. Defaults to empty.
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    /// Citation identifiers backing the finding. Defaults to empty.
    #[serde(default)]
    pub citations: Vec<String>,
    /// Suggested-fix identifiers associated with the finding. Defaults to empty.
    #[serde(default)]
    pub suggested_fixes: Vec<String>,
}

/// Finding severity. Mirrors Cerberus `Severity` (snake_case on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Minor,
    Major,
    Critical,
}

/// Where a finding points in the reviewed change. Mirrors Cerberus `Anchor`.
///
/// All location fields are optional: a `file` anchor carries only `path`, an
/// `inline` anchor adds `line`/`start_line`/`end_line`. Cerberus's
/// `hunk_digest` is not consumed by the eval and is ignored when present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    /// What the anchor points at.
    pub kind: AnchorKind,
    /// Repo-relative file path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Primary line number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// First line of a range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    /// Last line of a range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
}

/// Anchor kind. Mirrors Cerberus `AnchorKind` (snake_case on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorKind {
    Inline,
    File,
    Change,
    Run,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_parses_snake_case() {
        let s: Severity = serde_json::from_str("\"critical\"").unwrap();
        assert_eq!(s, Severity::Critical);
    }

    #[test]
    fn anchor_kind_parses_snake_case() {
        let k: AnchorKind = serde_json::from_str("\"file\"").unwrap();
        assert_eq!(k, AnchorKind::File);
    }

    #[test]
    fn finding_deserializes_with_inline_anchor() {
        let json = r#"{
            "id": "F1",
            "severity": "minor",
            "category": "security",
            "title": "t",
            "description": "d",
            "evidence": "e",
            "confidence": 0.8,
            "anchors": [{ "kind": "inline", "path": "src/x.rs", "line": 10, "start_line": 8, "end_line": 12 }]
        }"#;
        let f: Finding = serde_json::from_str(json).unwrap();
        assert_eq!(f.id, "F1");
        assert_eq!(f.severity, Severity::Minor);
        assert_eq!(f.confidence, 0.8);
        assert_eq!(f.anchors.len(), 1);
        let a = &f.anchors[0];
        assert_eq!(a.kind, AnchorKind::Inline);
        assert_eq!(a.path.as_deref(), Some("src/x.rs"));
        assert_eq!(a.line, Some(10));
        assert_eq!(a.start_line, Some(8));
        assert_eq!(a.end_line, Some(12));
    }

    #[test]
    fn finding_defaults_optional_collections() {
        let json = r#"{
            "id": "F2",
            "severity": "info",
            "category": "style",
            "title": "t",
            "description": "d",
            "evidence": "e",
            "confidence": 0.1
        }"#;
        let f: Finding = serde_json::from_str(json).unwrap();
        assert!(f.anchors.is_empty());
        assert!(f.citations.is_empty());
        assert!(f.suggested_fixes.is_empty());
    }

    #[test]
    fn artifact_defaults_findings_when_absent() {
        let json = r#"{ "schema_version": "cerberus.review_artifact.v1" }"#;
        let a = ReviewArtifact::from_json_str(json).unwrap();
        assert!(a.findings.is_empty());
    }

    #[test]
    fn artifact_ignores_unknown_envelope_fields() {
        let json = r#"{
            "schema_version": "v1",
            "artifact_id": "x",
            "verdict": "WARN",
            "summary": { "title": "t", "body": "b" },
            "findings": []
        }"#;
        let a = ReviewArtifact::from_json_str(json).unwrap();
        assert_eq!(a.schema_version, "v1");
        assert!(a.findings.is_empty());
    }

    #[test]
    fn from_path_reports_missing_file() {
        let err = ReviewArtifact::from_path("/no/such/crucible/artifact.json").unwrap_err();
        assert!(matches!(err, Error::Read { .. }));
    }
}
