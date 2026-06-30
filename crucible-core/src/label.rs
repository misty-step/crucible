//! Append-only finding judgments.
//!
//! A [`Label`] is one judgment of one finding: a correctness [`Verdict`] and an
//! orthogonal scope [`Disposition`] (reusing [`crate::adjudication`], never
//! redefining them), plus the conditions the judgment was made under —
//! `latency_ms` and `saw_grader_before_commit` — which backlog 005 needs to
//! decide whether a label is valid *calibration* data. A blind, snap judgment
//! and a slow, grader-revealed one are different evidence; the queue doubles as
//! calibration only if it records which one it was.
//!
//! Labels are **append-only** by contract: a correction is a new [`Label`] with
//! a later `timestamp`, never a mutation of an existing one, so the full
//! judgment history — and its calibration validity — is preserved. This module
//! is deliberately only the record type. It ships no mutable store, no index,
//! and no queue: a queue is a *view* over labels (backlog 004), not a third
//! store. The timestamp is caller-supplied; nothing here reads the clock.

use serde::{Deserialize, Serialize};

use crate::{Disposition, Verdict};

/// Schema identifier for a persisted [`Label`].
pub const LABEL_SCHEMA: &str = "crucible.label.v1";

/// One append-only judgment of a single finding.
///
/// See the [module docs](self) for the append-only contract and why the
/// calibration-validity conditions ride on every label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    /// Schema identifier; defaults to [`LABEL_SCHEMA`]. A present value is
    /// validated on load — an unknown schema is rejected, not assumed v1.
    #[serde(
        default = "label_schema",
        deserialize_with = "deserialize_label_schema"
    )]
    pub schema_version: String,
    /// The finding this judgment is about, by its artifact-stable id (e.g. `F1`).
    pub finding_id: String,
    /// The correctness verdict.
    pub verdict: Verdict,
    /// The scope disposition, orthogonal to the verdict.
    pub disposition: Disposition,
    /// Milliseconds from card presentation to commit — a calibration-validity
    /// signal (backlog 005). Defaults to `0` ("unmeasured").
    #[serde(default)]
    pub latency_ms: u64,
    /// Whether the grader's verdict was visible before this judgment committed.
    /// Blind judgments (`false`) are valid calibration data; revealed ones
    /// (`true`) are not. Defaults to `false` (blind).
    #[serde(default)]
    pub saw_grader_before_commit: bool,
    /// Caller-supplied RFC 3339 commit timestamp. Defaults to empty; never read
    /// from the clock.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timestamp: String,
}

fn label_schema() -> String {
    LABEL_SCHEMA.to_string()
}

fn deserialize_label_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, LABEL_SCHEMA)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_round_trips_with_verdict_and_disposition() {
        let label = Label {
            schema_version: LABEL_SCHEMA.to_string(),
            finding_id: "F1".to_string(),
            verdict: Verdict::Keep,
            disposition: Disposition { in_scope: true },
            latency_ms: 1234,
            saw_grader_before_commit: false,
            timestamp: "2026-06-29T12:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&label).unwrap();
        // The reused verdict/disposition keep their own wire shapes.
        assert!(json.contains(r#""verdict":"keep""#), "{json}");
        assert!(
            json.contains(r#""disposition":{"in_scope":true}"#),
            "{json}"
        );
        let back: Label = serde_json::from_str(&json).unwrap();
        assert_eq!(label, back);
    }

    #[test]
    fn label_defaults_calibration_fields_and_schema() {
        // A minimal label — just the finding and its verdict/disposition — must
        // load with blind, unmeasured defaults so older records still parse.
        let json = r#"{
            "finding_id": "F7",
            "verdict": "noise",
            "disposition": { "in_scope": false }
        }"#;
        let label: Label = serde_json::from_str(json).unwrap();
        assert_eq!(label.schema_version, LABEL_SCHEMA);
        assert_eq!(label.finding_id, "F7");
        assert_eq!(label.verdict, Verdict::Noise);
        assert!(!label.disposition.in_scope);
        assert_eq!(label.latency_ms, 0);
        assert!(!label.saw_grader_before_commit);
        assert!(label.timestamp.is_empty());
    }

    #[test]
    fn revealed_judgment_is_recorded_as_invalid_calibration_data() {
        // saw_grader_before_commit = true marks a label that cannot count toward
        // calibration; it must survive the round-trip intact.
        let label = Label {
            schema_version: LABEL_SCHEMA.to_string(),
            finding_id: "F3".to_string(),
            verdict: Verdict::Wrong,
            disposition: Disposition { in_scope: true },
            latency_ms: 50,
            saw_grader_before_commit: true,
            timestamp: String::new(),
        };
        let back: Label = serde_json::from_str(&serde_json::to_string(&label).unwrap()).unwrap();
        assert_eq!(label, back);
        assert!(back.saw_grader_before_commit);
    }

    #[test]
    fn unknown_schema_version_is_rejected() {
        // schema_version is no longer decorative: a garbage tag fails to load
        // rather than being silently accepted as v1.
        let json = r#"{
            "schema_version": "crucible.label.v999",
            "finding_id": "F1",
            "verdict": "keep",
            "disposition": { "in_scope": true }
        }"#;
        let err = serde_json::from_str::<Label>(json).unwrap_err();
        assert!(
            err.to_string().contains("schema_version"),
            "error should name the bad schema_version: {err}"
        );
    }
}
