//! Reproducibility records: how a verdict was produced, persisted so it can be
//! re-run with zero chat context.
//!
//! Backlog 003 (child 1) requires every run to persist an evaluation card
//! carrying enough to reproduce its verdict: the model and version, the sampling
//! temperature and seed count, the prompt and rubric hashes, the fixtures it
//! scored, the cost, and a timestamp. [`Provenance`] is that reproducibility
//! kernel; [`EvaluationCard`] is the top-level persisted artifact that wraps it
//! with the run-level cost, timing, and a `schema_version`.
//!
//! Two contracts keep a card *reproducible* rather than merely descriptive. The
//! hashes are stored, never computed here — the caller hashes the exact
//! prompt/rubric text and hands the digest in. The timestamp is likewise
//! supplied by the caller; nothing in this module reads the clock. So a card
//! rebuilt from identical inputs is byte-identical, and reproducibility is a
//! property of the recorded data, not of when the struct happened to be built.
//!
//! No CLI subcommand emits a card yet: it is exported now as the durable
//! reproducibility schema a run records and epic 005 (and Daedalus, via Harbor)
//! reads back, so the wire shape is fixed before there is a writer — part of
//! backlog 004's persisted-artifact contract.

use serde::{Deserialize, Serialize};

use crate::FixtureRef;

/// Schema identifier for a persisted [`EvaluationCard`].
pub const EVALUATION_CARD_SCHEMA: &str = "crucible.evaluation_card.v1";
/// Schema identifier for a persisted [`RunRecord`].
pub const RUN_RECORD_SCHEMA: &str = "crucible.run_record.v1";

/// The reproducibility kernel: everything needed to re-run a judgment and get
/// the same verdict.
///
/// Embedded in an [`EvaluationCard`]; not a standalone persisted artifact, so it
/// carries no `schema_version` of its own. `model` and the sampling parameters
/// are required — without them there is nothing to reproduce. The hashes and
/// fixtures default to empty so a partial provenance (e.g. a deterministic
/// grader with no rubric) still serializes cleanly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// The judge/model identifier, e.g. `anthropic/claude-opus-4`.
    pub model: String,
    /// The pinned model version/build, when distinct from `model`. Defaults to
    /// empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model_version: String,
    /// Sampling temperature.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub temperature: f64,
    /// Number of seeds / samples drawn per item.
    pub seed_count: u32,
    /// Hash of the exact prompt text — computed by the caller, stored verbatim.
    /// Defaults to empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt_hash: String,
    /// Hash of the exact rubric text — computed by the caller, stored verbatim.
    /// Defaults to empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rubric_hash: String,
    /// The fixtures this run scored, by content hash. Defaults to empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixture_refs: Vec<FixtureRef>,
}

/// A persisted evaluation card: a [`Provenance`] kernel plus run-level cost,
/// timing, and a schema tag.
///
/// The top-level artifact backlog 003 requires — it "reproduces the verdict with
/// zero chat context." `timestamp` is a caller-supplied RFC 3339 string: the
/// card never calls the clock, so two cards built from the same run are equal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationCard {
    /// Schema identifier; defaults to [`EVALUATION_CARD_SCHEMA`]. A present value
    /// is validated on load — an unknown schema is rejected, not assumed v1.
    #[serde(
        default = "evaluation_card_schema",
        deserialize_with = "deserialize_evaluation_card_schema"
    )]
    pub schema_version: String,
    /// How the verdict was produced — the reproducibility kernel.
    pub provenance: Provenance,
    /// Cost of the run in US dollars. Defaults to `0.0`.
    #[serde(default, serialize_with = "crate::serde_util::serialize_finite")]
    pub cost_usd: f64,
    /// When the run completed, as a caller-supplied RFC 3339 timestamp. Defaults
    /// to empty; never read from the clock.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub timestamp: String,
}

/// The score shape persisted with a [`RunRecord`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunScore {
    /// Metric id, e.g. `prompt_rubric_pass_rate`.
    pub metric: String,
    /// Successful items in the aggregate.
    pub successes: u64,
    /// Denominator for the aggregate.
    pub n: u64,
    /// Point estimate. `None` means no denominator/no data.
    #[serde(serialize_with = "crate::serde_util::serialize_finite_option")]
    pub point: Option<f64>,
    /// Lower confidence bound.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub lower: f64,
    /// Upper confidence bound.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub upper: f64,
    /// Confidence level for the interval.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub confidence: f64,
    /// Interval method, e.g. `Wilson`.
    pub method: String,
}

/// A durable run record: the benchmark/config identity, score, artifact
/// pointers, and the reproducibility card needed to re-run or audit the verdict.
///
/// This is the stable JSON artifact that the SQLite ledger materializes from its
/// normalized rows. It intentionally stores artifact pointers rather than raw
/// diffs or model transcripts; raw content stays under the ignored run tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// Schema identifier; defaults to [`RUN_RECORD_SCHEMA`]. Unknown versions
    /// are rejected on load.
    #[serde(
        default = "run_record_schema",
        deserialize_with = "deserialize_run_record_schema"
    )]
    pub schema_version: String,
    /// Stable run id inside the run ledger.
    pub run_id: String,
    /// Benchmark/eval id.
    pub benchmark_id: String,
    /// Config id selected by the runner/store.
    pub config_id: String,
    /// Runner family, e.g. `prompt_benchmark` or `key_recall`.
    pub runner_kind: String,
    /// Output directory that holds the raw evidence packet.
    pub output_dir: String,
    /// `run-report.json` path for the invocation.
    pub run_report: String,
    /// Primary runner evidence path, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_path: Option<String>,
    /// Eval spec path, when this came from a declared spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_path: Option<String>,
    /// Artifact pointers associated with this run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    /// Aggregate score with uncertainty.
    pub score: RunScore,
    /// Reproducibility card for the verdict.
    pub evaluation_card: EvaluationCard,
}

fn evaluation_card_schema() -> String {
    EVALUATION_CARD_SCHEMA.to_string()
}

fn run_record_schema() -> String {
    RUN_RECORD_SCHEMA.to_string()
}

fn deserialize_evaluation_card_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, EVALUATION_CARD_SCHEMA)
}

fn deserialize_run_record_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, RUN_RECORD_SCHEMA)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_provenance() -> Provenance {
        Provenance {
            model: "anthropic/claude-opus-4".to_string(),
            model_version: "claude-opus-4-8".to_string(),
            temperature: 0.0,
            seed_count: 3,
            prompt_hash: "sha256:prompt".to_string(),
            rubric_hash: "sha256:rubric".to_string(),
            fixture_refs: vec![FixtureRef("sha256:fix1".to_string())],
        }
    }

    #[test]
    fn evaluation_card_round_trips() {
        let card = EvaluationCard {
            schema_version: EVALUATION_CARD_SCHEMA.to_string(),
            provenance: sample_provenance(),
            cost_usd: 0.42,
            timestamp: "2026-06-29T12:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&card).unwrap();
        let back: EvaluationCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card, back);
        // Fixtures ride along as bare hash strings.
        assert!(
            json.contains(r#""fixture_refs":["sha256:fix1"]"#),
            "fixtures not embedded as bare hashes: {json}"
        );
    }

    #[test]
    fn card_built_from_same_inputs_is_identical() {
        // No clock is read anywhere; the timestamp is data, so two cards from the
        // same run are byte-identical — the reproducibility contract.
        let a = EvaluationCard {
            schema_version: EVALUATION_CARD_SCHEMA.to_string(),
            provenance: sample_provenance(),
            cost_usd: 0.42,
            timestamp: "2026-06-29T12:00:00Z".to_string(),
        };
        let b = EvaluationCard {
            schema_version: EVALUATION_CARD_SCHEMA.to_string(),
            provenance: sample_provenance(),
            cost_usd: 0.42,
            timestamp: "2026-06-29T12:00:00Z".to_string(),
        };
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn card_defaults_schema_and_optional_fields() {
        // A card with only the reproducibility kernel must load: schema defaults,
        // cost is 0.0, timestamp empty.
        let json = r#"{
            "provenance": {
                "model": "anthropic/claude-opus-4",
                "temperature": 0.2,
                "seed_count": 1
            }
        }"#;
        let card: EvaluationCard = serde_json::from_str(json).unwrap();
        assert_eq!(card.schema_version, EVALUATION_CARD_SCHEMA);
        assert_eq!(card.cost_usd, 0.0);
        assert!(card.timestamp.is_empty());
        // Optional provenance fields default to empty without erroring.
        assert!(card.provenance.model_version.is_empty());
        assert!(card.provenance.prompt_hash.is_empty());
        assert!(card.provenance.fixture_refs.is_empty());
    }

    #[test]
    fn minimal_provenance_skips_empty_optionals_on_the_wire() {
        let prov = Provenance {
            model: "anthropic/claude-opus-4".to_string(),
            model_version: String::new(),
            temperature: 0.7,
            seed_count: 1,
            prompt_hash: String::new(),
            rubric_hash: String::new(),
            fixture_refs: Vec::new(),
        };
        let json = serde_json::to_string(&prov).unwrap();
        assert_eq!(
            json,
            r#"{"model":"anthropic/claude-opus-4","temperature":0.7,"seed_count":1}"#
        );
        let back: Provenance = serde_json::from_str(&json).unwrap();
        assert_eq!(prov, back);
    }

    #[test]
    fn non_finite_cost_or_temperature_is_refused() {
        // A non-finite cost/temperature would serialize to a null that fails to
        // read back as f64; serialization must error instead.
        let mut card = EvaluationCard {
            schema_version: EVALUATION_CARD_SCHEMA.to_string(),
            provenance: sample_provenance(),
            cost_usd: 0.42,
            timestamp: String::new(),
        };
        card.cost_usd = f64::NAN;
        assert!(
            serde_json::to_string(&card).is_err(),
            "a NaN cost must not serialize to a non-round-tripping null"
        );
        card.cost_usd = 0.42;
        card.provenance.temperature = f64::INFINITY;
        assert!(
            serde_json::to_string(&card).is_err(),
            "a non-finite temperature must not serialize"
        );

        let record = RunRecord {
            schema_version: RUN_RECORD_SCHEMA.to_string(),
            run_id: "run-1".to_string(),
            benchmark_id: "bench".to_string(),
            config_id: "cfg".to_string(),
            runner_kind: "deterministic".to_string(),
            output_dir: "runs/local/bench".to_string(),
            run_report: "runs/local/bench/run-report.json".to_string(),
            evidence_path: None,
            spec_path: None,
            artifacts: Vec::new(),
            score: RunScore {
                metric: "m".to_string(),
                successes: 0,
                n: 0,
                point: Some(f64::NAN),
                lower: 0.0,
                upper: 0.0,
                confidence: 0.95,
                method: "Wilson".to_string(),
            },
            evaluation_card: EvaluationCard {
                schema_version: EVALUATION_CARD_SCHEMA.to_string(),
                provenance: sample_provenance(),
                cost_usd: 0.0,
                timestamp: String::new(),
            },
        };
        assert!(
            serde_json::to_string(&record).is_err(),
            "a non-finite score point must not serialize"
        );
    }

    #[test]
    fn unknown_schema_version_is_rejected() {
        let json = r#"{
            "schema_version": "crucible.evaluation_card.v999",
            "provenance": {
                "model": "anthropic/claude-opus-4",
                "temperature": 0.2,
                "seed_count": 1
            }
        }"#;
        let err = serde_json::from_str::<EvaluationCard>(json).unwrap_err();
        assert!(
            err.to_string().contains("schema_version"),
            "error should name the bad schema_version: {err}"
        );
    }

    #[test]
    fn run_record_wraps_score_artifacts_and_card() {
        let record = RunRecord {
            schema_version: RUN_RECORD_SCHEMA.to_string(),
            run_id: "run-1:prompt-smoke-v0".to_string(),
            benchmark_id: "prompt-smoke-v0".to_string(),
            config_id: "prompt:open_router:openrouter/auto:fnv1a64:prompt".to_string(),
            runner_kind: "prompt_benchmark".to_string(),
            output_dir: "runs/local/prompt-smoke".to_string(),
            run_report: "runs/local/prompt-smoke/run-report.json".to_string(),
            evidence_path: Some("runs/local/prompt-smoke/prompt-run.json".to_string()),
            spec_path: Some("evals/prompt-smoke-v0.json".to_string()),
            artifacts: vec![
                "evals/prompt-smoke-v0.json".to_string(),
                "runs/local/prompt-smoke/prompt-run.json".to_string(),
            ],
            score: RunScore {
                metric: "prompt_rubric_pass_rate".to_string(),
                successes: 1,
                n: 1,
                point: Some(1.0),
                lower: 0.2,
                upper: 1.0,
                confidence: 0.95,
                method: "Wilson".to_string(),
            },
            evaluation_card: EvaluationCard {
                schema_version: EVALUATION_CARD_SCHEMA.to_string(),
                provenance: sample_provenance(),
                cost_usd: 0.42,
                timestamp: "2026-07-01T12:00:00Z".to_string(),
            },
        };

        let json = serde_json::to_string(&record).unwrap();
        let back: RunRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record, back);
        assert!(json.contains("crucible.run_record.v1"));
        assert!(json.contains("crucible.evaluation_card.v1"));
    }

    #[test]
    fn unknown_run_record_schema_version_is_rejected() {
        let json = r#"{
            "schema_version": "crucible.run_record.v999",
            "run_id": "run-1",
            "benchmark_id": "bench",
            "config_id": "cfg",
            "runner_kind": "deterministic",
            "output_dir": "runs/local/bench",
            "run_report": "runs/local/bench/run-report.json",
            "score": {
                "metric": "m",
                "successes": 0,
                "n": 0,
                "point": null,
                "lower": 0.0,
                "upper": 0.0,
                "confidence": 0.95,
                "method": "Wilson"
            },
            "evaluation_card": {
                "provenance": {
                    "model": "deterministic",
                    "temperature": 0.0,
                    "seed_count": 1
                }
            }
        }"#;
        let err = serde_json::from_str::<RunRecord>(json).unwrap_err();
        assert!(
            err.to_string().contains("schema_version"),
            "error should name the bad schema_version: {err}"
        );
    }
}
