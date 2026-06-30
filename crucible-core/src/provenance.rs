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

fn evaluation_card_schema() -> String {
    EVALUATION_CARD_SCHEMA.to_string()
}

fn deserialize_evaluation_card_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, EVALUATION_CARD_SCHEMA)
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
}
