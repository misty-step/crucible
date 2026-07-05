//! Judge calibration records: the measured agreement that gates a model judge.
//!
//! Backlog 003 forbids an unsupervised model/agentic judge until it clears a
//! measured judge-vs-human agreement on a κ-validated set. A
//! [`CalibrationRecord`] is the durable evidence of that gate: the judge's id,
//! the number of paired items, raw [`agreement`](crate::agreement) and
//! chance-corrected [`cohen_kappa`](crate::cohen_kappa), the [`ConfusionMatrix`]
//! of its calls against the human reference, the unlock threshold, and whether
//! it unlocked.
//!
//! This module *records* — it does not *compute*. The agreement, κ, and
//! confusion are produced by [`crate::measure`] and the grading pipeline; the
//! record stores their outputs so a judge's licence to score is auditable from
//! the artifact alone, with no need to re-derive it. `unlocked` is likewise the
//! decision *as made* against the data present at calibration time, preserved —
//! not recomputed from the threshold at read time.
//!
//! No CLI subcommand emits a record yet: it is exported now as the durable
//! judge-gate schema that epic 005 (the phone-adjudication calibration loop) and
//! Daedalus read, so the wire shape is fixed (schema-tagged, serde round-tripped)
//! before there is a writer — part of backlog 004's persisted-artifact contract.

use serde::{Deserialize, Serialize};

/// Schema identifier for a persisted [`CalibrationRecord`].
pub const CALIBRATION_RECORD_SCHEMA: &str = "crucible.calibration_record.v1";

/// Extract the model-family identifier from a provider model slug: the
/// segment before the first `/` (e.g. `"openai/gpt-4o-mini"` -> `"openai"`).
/// A slug with no `/` is its own family.
///
/// This is a coarse, deterministic proxy for "same or closely related model
/// family" (report §6's self-preference bias check: "judge prefers outputs
/// from same model family") — it catches the common case (judge and
/// candidate generator both slugged under the same provider/org) without
/// attempting semantic reasoning about which providers rebrand which
/// underlying models. A judge and generator that pass this check as
/// "different family" may still share ancestry the slug doesn't reveal; this
/// is a floor, not a proof of independence.
pub fn model_family(model: &str) -> &str {
    model.split('/').next().unwrap_or(model)
}

/// Whether two model slugs share a [`model_family`] — the self-preference
/// bias risk report §6 names. Case-sensitive: provider slugs are
/// conventionally lowercase, so a caller feeding inconsistent casing needs to
/// normalize first rather than relying on this to fold case.
pub fn shares_model_family(a: &str, b: &str) -> bool {
    model_family(a) == model_family(b)
}

/// Stable identity for a judge's calibration state: unique per (judge model,
/// judge system prompt, calibration rubric set). This is what makes
/// calibration state "invalidated when judge model/prompt/rubric changes"
/// mechanical rather than a separate check to remember: any change to one of
/// the three inputs yields a different key, so looking up the new key simply
/// finds no prior licence — locked/unlicensed until a run under the new key
/// establishes one. See [`CalibrationRecord::licence_key`].
pub fn judge_licence_key(judge_model: &str, system_prompt_hash: &str, rubric_hash: &str) -> String {
    format!("judge-licence:v1:{judge_model}:{system_prompt_hash}:{rubric_hash}")
}

/// A rate in `[0.0, 1.0]` — `numerator / denominator`, defined as `0.0` when
/// the denominator is `0` (no cases to rate) rather than `NaN`. Mirrors
/// [`crate::cohen_kappa`]'s "degenerate input records as `0.0`, never a
/// silent non-finite" convention.
fn safe_rate(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

/// A 2×2 confusion of a judge's binary calls against a human reference.
///
/// Counts only — the agreement and κ derived from them live on the
/// [`CalibrationRecord`]. "Positive" means the rater marked the finding (e.g.
/// `keep`); "negative" means it did not. Every count defaults to `0` so a
/// partial or absent matrix still loads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfusionMatrix {
    /// Judge positive, human positive.
    #[serde(default)]
    pub true_positive: u64,
    /// Judge positive, human negative.
    #[serde(default)]
    pub false_positive: u64,
    /// Judge negative, human positive.
    #[serde(default)]
    pub false_negative: u64,
    /// Judge negative, human negative.
    #[serde(default)]
    pub true_negative: u64,
}

impl ConfusionMatrix {
    /// False-positive rate: of the human-negative cases, the fraction the
    /// judge called positive. `0.0` when there are no human-negative cases to
    /// rate (never `NaN`).
    pub fn false_positive_rate(&self) -> f64 {
        safe_rate(
            self.false_positive,
            self.false_positive + self.true_negative,
        )
    }

    /// False-negative rate: of the human-positive cases, the fraction the
    /// judge called negative. `0.0` when there are no human-positive cases to
    /// rate (never `NaN`).
    pub fn false_negative_rate(&self) -> f64 {
        safe_rate(
            self.false_negative,
            self.false_negative + self.true_positive,
        )
    }
}

/// A judge's calibration record: the measured agreement that licenses its use.
///
/// See the [module docs](self): this records the [`crate::measure`] outputs and
/// the unlock decision, it does not recompute them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationRecord {
    /// Schema identifier; defaults to [`CALIBRATION_RECORD_SCHEMA`]. A present
    /// value is validated on load — an unknown schema is rejected, not assumed v1.
    #[serde(
        default = "calibration_record_schema",
        deserialize_with = "deserialize_calibration_schema"
    )]
    pub schema_version: String,
    /// The judge being calibrated (model/config id).
    pub judge_id: String,
    /// Number of paired (judge, human) items the calibration is measured over.
    pub n: u64,
    /// Raw percent agreement, from [`crate::agreement`].
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub agreement: f64,
    /// Chance-corrected agreement (Cohen's κ), from [`crate::cohen_kappa`].
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub cohen_kappa: f64,
    /// The judge-vs-human confusion of binary calls. Defaults to all-zero.
    #[serde(default)]
    pub confusion: ConfusionMatrix,
    /// [`ConfusionMatrix::false_positive_rate`], carried as its own named
    /// field (report §6 item 7 / §11: "For model-as-judge results,
    /// include... False-positive rate") rather than requiring a consumer to
    /// re-derive it from `confusion`. Defaults to `0.0`.
    #[serde(default, serialize_with = "crate::serde_util::serialize_finite")]
    pub false_positive_rate: f64,
    /// [`ConfusionMatrix::false_negative_rate`], carried as its own named
    /// field (report §6 item 7 / §11: "...False-negative rate"). Defaults to
    /// `0.0`.
    #[serde(default, serialize_with = "crate::serde_util::serialize_finite")]
    pub false_negative_rate: f64,
    /// Calibration probes the judge answered `UNKNOWN` on — diagnostic, not
    /// counted in `n`/`agreement`/`confusion`, but reported so a judge that
    /// hedges on every hard case doesn't read as perfectly calibrated on the
    /// cases it did commit to. Defaults to `0`.
    #[serde(default)]
    pub unknown_count: u64,
    /// The model that generated the candidate outputs this judge scored,
    /// when known (self-evaluation bias check, report §6). `None` when the
    /// generator is unrecorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_id: Option<String>,
    /// Whether `judge_id` and `generator_id` share a [`model_family`] — the
    /// self-preference bias risk report §6 names ("judge prefers outputs
    /// from same model family", mitigation "use diverse judges; calibrate
    /// against human labels"). `false` when the generator is unknown or a
    /// different family — this field only ever *surfaces* the risk, it never
    /// blocks the run; the judge-gaming canary is the only hard refusal.
    /// Defaults to `false`.
    #[serde(default)]
    pub self_evaluation_bias_risk: bool,
    /// The agreement (or κ) threshold the judge had to clear to unlock.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub unlock_threshold: f64,
    /// Whether the judge cleared the gate and may score unsupervised. Defaults to
    /// `false` (locked).
    #[serde(default)]
    pub unlocked: bool,
    /// [`judge_licence_key`] for this measurement: the standing identity a
    /// caller queries across runs to ask "is this judge (this model, this
    /// prompt, this rubric set) currently licensed" without recomputing
    /// calibration from scratch. Defaults to empty for records predating
    /// this field — an empty key matches no licence lookup, which is the
    /// safe (locked/unknown) direction.
    #[serde(default)]
    pub licence_key: String,
}

fn calibration_record_schema() -> String {
    CALIBRATION_RECORD_SCHEMA.to_string()
}

fn deserialize_calibration_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, CALIBRATION_RECORD_SCHEMA)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confusion_matrix_defaults_to_all_zero() {
        let empty: ConfusionMatrix = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, ConfusionMatrix::default());
        assert_eq!(empty.true_positive, 0);
        assert_eq!(empty.true_negative, 0);
    }

    #[test]
    fn calibration_record_round_trips() {
        let rec = CalibrationRecord {
            schema_version: CALIBRATION_RECORD_SCHEMA.to_string(),
            judge_id: "claude-judge".to_string(),
            n: 50,
            agreement: 0.84,
            cohen_kappa: 0.62,
            confusion: ConfusionMatrix {
                true_positive: 20,
                false_positive: 4,
                false_negative: 4,
                true_negative: 22,
            },
            false_positive_rate: 0.15,
            false_negative_rate: 0.2,
            unknown_count: 2,
            generator_id: Some("claude-generator".to_string()),
            self_evaluation_bias_risk: false,
            unlock_threshold: 0.6,
            unlocked: true,
            licence_key: "judge-licence:v1:claude-judge:hash1:hash2".to_string(),
        };
        let json = serde_json::to_string(&rec).unwrap();
        let back: CalibrationRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(rec, back);
        assert!(back.unlocked);
        assert_eq!(back.confusion.true_positive, 20);
        assert_eq!(back.unknown_count, 2);
        assert_eq!(back.generator_id.as_deref(), Some("claude-generator"));
        assert_eq!(
            back.licence_key,
            "judge-licence:v1:claude-judge:hash1:hash2"
        );
    }

    #[test]
    fn record_defaults_lock_and_confusion() {
        // A judge measured below threshold: confusion may be absent, and an
        // omitted `unlocked` must default to locked — never silently unlock.
        let json = r#"{
            "judge_id": "claude-judge",
            "n": 12,
            "agreement": 0.5,
            "cohen_kappa": 0.1,
            "unlock_threshold": 0.6
        }"#;
        let rec: CalibrationRecord = serde_json::from_str(json).unwrap();
        assert_eq!(rec.schema_version, CALIBRATION_RECORD_SCHEMA);
        assert_eq!(rec.confusion, ConfusionMatrix::default());
        assert!(
            !rec.unlocked,
            "an omitted unlock flag must default to locked"
        );
        assert_eq!(
            rec.false_positive_rate, 0.0,
            "an omitted FP rate defaults to 0.0"
        );
        assert_eq!(
            rec.false_negative_rate, 0.0,
            "an omitted FN rate defaults to 0.0"
        );
        assert_eq!(rec.unknown_count, 0);
        assert_eq!(rec.generator_id, None);
        assert!(
            !rec.self_evaluation_bias_risk,
            "an omitted bias-risk flag must default to false, not a silent true"
        );
        assert_eq!(rec.licence_key, "");
    }

    #[test]
    fn non_finite_metric_is_refused_not_silently_nulled() {
        // A NaN/∞ agreement would serialize to JSON `null` and then fail to read
        // back as f64 — a silent round-trip break. Serialization must refuse it.
        let mut rec = CalibrationRecord {
            schema_version: CALIBRATION_RECORD_SCHEMA.to_string(),
            judge_id: "claude-judge".to_string(),
            n: 10,
            agreement: 0.8,
            cohen_kappa: 0.6,
            confusion: ConfusionMatrix::default(),
            false_positive_rate: 0.1,
            false_negative_rate: 0.1,
            unknown_count: 0,
            generator_id: None,
            self_evaluation_bias_risk: false,
            unlock_threshold: 0.6,
            unlocked: false,
            licence_key: "judge-licence:v1:claude-judge:hash1:hash2".to_string(),
        };
        for set in [
            |r: &mut CalibrationRecord| r.agreement = f64::NAN,
            |r: &mut CalibrationRecord| r.cohen_kappa = f64::INFINITY,
            |r: &mut CalibrationRecord| r.unlock_threshold = f64::NEG_INFINITY,
            |r: &mut CalibrationRecord| r.false_positive_rate = f64::NAN,
            |r: &mut CalibrationRecord| r.false_negative_rate = f64::INFINITY,
        ] {
            let mut bad = rec.clone();
            set(&mut bad);
            assert!(
                serde_json::to_string(&bad).is_err(),
                "a non-finite metric must error, not serialize to a null that won't round-trip"
            );
        }
        // The valid record still serializes fine.
        rec.agreement = 0.84;
        assert!(serde_json::to_string(&rec).is_ok());
    }

    #[test]
    fn unknown_schema_version_is_rejected() {
        // schema_version is no longer decorative: an unknown tag fails to load
        // rather than being silently treated as v1.
        let json = r#"{
            "schema_version": "crucible.calibration_record.v999",
            "judge_id": "claude-judge",
            "n": 10,
            "agreement": 0.8,
            "cohen_kappa": 0.6,
            "unlock_threshold": 0.6
        }"#;
        let err = serde_json::from_str::<CalibrationRecord>(json).unwrap_err();
        assert!(
            err.to_string().contains("schema_version"),
            "error should name the bad schema_version: {err}"
        );
    }

    // ---- self-evaluation bias (model family) -------------------------------

    #[test]
    fn model_family_is_the_slug_segment_before_the_first_slash() {
        assert_eq!(model_family("openai/gpt-4o-mini"), "openai");
        assert_eq!(model_family("anthropic/claude-opus-4"), "anthropic");
        assert_eq!(
            model_family("no-slash-slug"),
            "no-slash-slug",
            "a slug with no provider prefix is its own family"
        );
    }

    #[test]
    fn shares_model_family_matches_same_provider_prefix_only() {
        assert!(shares_model_family("openai/gpt-4o-mini", "openai/gpt-4o"));
        assert!(!shares_model_family(
            "openai/gpt-4o-mini",
            "anthropic/claude-opus-4"
        ));
        assert!(
            !shares_model_family("openai/gpt-4o", "OpenAI/gpt-4o"),
            "family matching is case-sensitive; callers normalize first"
        );
    }

    #[test]
    fn judge_licence_key_changes_with_any_of_its_three_inputs() {
        let base = judge_licence_key("openai/gpt-4o", "prompt-hash-1", "rubric-hash-1");
        assert_ne!(
            base,
            judge_licence_key("anthropic/claude-opus-4", "prompt-hash-1", "rubric-hash-1"),
            "a different judge model must yield a different licence key"
        );
        assert_ne!(
            base,
            judge_licence_key("openai/gpt-4o", "prompt-hash-2", "rubric-hash-1"),
            "a different judge prompt must yield a different licence key"
        );
        assert_ne!(
            base,
            judge_licence_key("openai/gpt-4o", "prompt-hash-1", "rubric-hash-2"),
            "a different rubric set must yield a different licence key"
        );
        assert_eq!(
            base,
            judge_licence_key("openai/gpt-4o", "prompt-hash-1", "rubric-hash-1"),
            "identical inputs are deterministic"
        );
    }

    // ---- FP/FN rate ----------------------------------------------------------

    #[test]
    fn confusion_matrix_rates_divide_by_the_relevant_actual_class() {
        let confusion = ConfusionMatrix {
            true_positive: 20,
            false_positive: 4,
            false_negative: 4,
            true_negative: 22,
        };
        // FPR: of the 26 actual negatives (4 FP + 22 TN), 4 were called positive.
        assert!((confusion.false_positive_rate() - (4.0 / 26.0)).abs() < 1e-9);
        // FNR: of the 24 actual positives (4 FN + 20 TP), 4 were called negative.
        assert!((confusion.false_negative_rate() - (4.0 / 24.0)).abs() < 1e-9);
    }

    #[test]
    fn confusion_matrix_rates_are_zero_not_nan_when_a_class_is_absent() {
        // No actual negatives (false_positive + true_negative == 0) and no
        // actual positives (false_negative + true_positive == 0): both rates
        // must be the safe 0.0, never a NaN that would fail to serialize.
        let confusion = ConfusionMatrix {
            true_positive: 0,
            false_positive: 0,
            false_negative: 0,
            true_negative: 0,
        };
        assert_eq!(confusion.false_positive_rate(), 0.0);
        assert_eq!(confusion.false_negative_rate(), 0.0);
    }
}
