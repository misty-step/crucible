//! Trace: the ordered record of *how* a run produced its verdict.
//!
//! Runs persist prompt/response, config, score, and cost, but not the
//! structured record of *how* the candidate got there: retrieved context,
//! tool calls, agent steps, intermediate reasoning artifacts. For the
//! code-review family this is tolerable (Cerberus's `ReviewArtifact`/
//! `Finding` already carries anchors); the next families VISION.md names —
//! Harness Kit primitive evals, agentic product behavior — need enough
//! structure to answer "why did this candidate fail" without re-running it.
//!
//! A [`Trace`] is the ordered [`TraceStep`] sequence a runner emits while it
//! executes one task/subject: a judge call, the verdict parsed out of the
//! response, an optional calibration check today (the agentic-judge runner);
//! a tool call, a retrieval, or an agent step for a future runner kind.
//! `kind`/`detail` are deliberately open (`String`/[`serde_json::Value`])
//! rather than a closed enum — Trace must generalize across runner kinds not
//! yet built without a schema migration for every new step shape, the same
//! reason the `crucible` binary crate's `EvalReport` carries free-text
//! `notes` rather than a closed set of note kinds.
//!
//! A [`Trace`] is persisted as its own JSON artifact and pointed to from the
//! run's evidence/`RunRecord` the same way `evidence_path`/`spec_path`
//! already are (see [`crate::RunRecord`]) — no parallel storage mechanism.

use serde::{Deserialize, Serialize};

/// Schema identifier for a persisted [`Trace`].
pub const TRACE_SCHEMA: &str = "crucible.trace.v1";

/// One ordered event in a run's execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceStep {
    /// Position in the trace, starting at `0`. The authoritative order —
    /// `timestamp` is diagnostic (may be coarse or absent), `sequence` never
    /// is.
    pub sequence: u64,
    /// Caller-supplied RFC 3339 timestamp, when the runner has wall-clock
    /// time to attach. Defaults to empty; nothing in this module reads the
    /// clock itself (the same discipline as [`crate::Provenance`]/
    /// [`crate::EvaluationCard`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timestamp: String,
    /// Step kind, e.g. `"judge_call"`, `"verdict_parsed"`, `"calibration_check"`.
    /// Open string rather than a closed enum — see the [module docs](self).
    pub kind: String,
    /// Human-readable label for this step, e.g. a task id. Defaults to empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Structured detail for this step, in the runner's own shape. Defaults
    /// to `null`.
    #[serde(default)]
    pub detail: serde_json::Value,
    /// This step's own outcome, when it has one, e.g. `"pass"`, `"fail"`,
    /// `"unknown"`, `"error"`. `None` for a purely informational step (e.g.
    /// recording that a call was made, before its result is known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// A run's ordered trace: enough structure to answer "why did this candidate
/// fail" without re-running it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    /// Schema identifier; defaults to [`TRACE_SCHEMA`]. A present value is
    /// validated on load — an unknown schema is rejected, not assumed v1.
    #[serde(
        default = "trace_schema",
        deserialize_with = "deserialize_trace_schema"
    )]
    pub schema_version: String,
    /// The task/subject this trace explains, e.g. an `AgenticJudgeTask::task_id`
    /// or a spec id for a run-level trace.
    pub subject_id: String,
    /// Ordered steps, `sequence`-ascending.
    #[serde(default)]
    pub steps: Vec<TraceStep>,
}

impl Trace {
    /// Steps whose `outcome` reads as a failure to decide/succeed —
    /// `"unknown"`, `"fail"`, or `"error"` — the steps worth reading first
    /// when a run's overall verdict was not a clean pass. Order is
    /// preserved from `steps`.
    pub fn failure_steps(&self) -> impl Iterator<Item = &TraceStep> {
        self.steps.iter().filter(|step| {
            matches!(
                step.outcome.as_deref(),
                Some("unknown") | Some("fail") | Some("error")
            )
        })
    }
}

fn trace_schema() -> String {
    TRACE_SCHEMA.to_string()
}

fn deserialize_trace_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, TRACE_SCHEMA)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_step(sequence: u64, kind: &str, outcome: Option<&str>) -> TraceStep {
        TraceStep {
            sequence,
            timestamp: "2026-07-04T12:00:00Z".to_string(),
            kind: kind.to_string(),
            label: "task-1".to_string(),
            detail: serde_json::json!({"note": "detail"}),
            outcome: outcome.map(str::to_string),
        }
    }

    #[test]
    fn trace_round_trips() {
        let trace = Trace {
            schema_version: TRACE_SCHEMA.to_string(),
            subject_id: "task-1".to_string(),
            steps: vec![
                sample_step(0, "judge_call", None),
                sample_step(1, "verdict_parsed", Some("unknown")),
            ],
        };
        let json = serde_json::to_string(&trace).unwrap();
        let back: Trace = serde_json::from_str(&json).unwrap();
        assert_eq!(trace, back);
    }

    #[test]
    fn steps_default_to_empty_and_schema_defaults() {
        let json = r#"{"subject_id": "task-1"}"#;
        let trace: Trace = serde_json::from_str(json).unwrap();
        assert_eq!(trace.schema_version, TRACE_SCHEMA);
        assert!(trace.steps.is_empty());
    }

    #[test]
    fn step_defaults_timestamp_label_detail_and_outcome() {
        let json = r#"{
            "subject_id": "task-1",
            "steps": [{"sequence": 0, "kind": "judge_call"}]
        }"#;
        let trace: Trace = serde_json::from_str(json).unwrap();
        let step = &trace.steps[0];
        assert_eq!(step.sequence, 0);
        assert_eq!(step.kind, "judge_call");
        assert!(
            step.timestamp.is_empty(),
            "an omitted timestamp defaults to empty, not a clock read"
        );
        assert!(step.label.is_empty());
        assert_eq!(step.detail, serde_json::Value::Null);
        assert!(step.outcome.is_none());
    }

    #[test]
    fn unknown_schema_version_is_rejected() {
        let json = r#"{
            "schema_version": "crucible.trace.v999",
            "subject_id": "task-1",
            "steps": []
        }"#;
        let err = serde_json::from_str::<Trace>(json).unwrap_err();
        assert!(
            err.to_string().contains("schema_version"),
            "error should name the bad schema_version: {err}"
        );
    }

    #[test]
    fn failure_steps_surfaces_unknown_fail_and_error_outcomes_only() {
        let trace = Trace {
            schema_version: TRACE_SCHEMA.to_string(),
            subject_id: "task-1".to_string(),
            steps: vec![
                sample_step(0, "judge_call", None),
                sample_step(1, "verdict_parsed", Some("pass")),
                sample_step(2, "verdict_parsed", Some("unknown")),
                sample_step(3, "calibration_check", Some("fail")),
                sample_step(4, "tool_call", Some("error")),
            ],
        };
        let failures: Vec<&str> = trace.failure_steps().map(|s| s.kind.as_str()).collect();
        assert_eq!(
            failures,
            vec!["verdict_parsed", "calibration_check", "tool_call"],
            "only unknown/fail/error outcomes surface, in original order"
        );
    }

    #[test]
    fn failure_steps_is_empty_for_an_all_pass_trace() {
        let trace = Trace {
            schema_version: TRACE_SCHEMA.to_string(),
            subject_id: "task-1".to_string(),
            steps: vec![
                sample_step(0, "judge_call", None),
                sample_step(1, "verdict_parsed", Some("pass")),
            ],
        };
        assert_eq!(trace.failure_steps().count(), 0);
    }
}
