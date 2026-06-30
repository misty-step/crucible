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
    /// The agreement (or κ) threshold the judge had to clear to unlock.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub unlock_threshold: f64,
    /// Whether the judge cleared the gate and may score unsupervised. Defaults to
    /// `false` (locked).
    #[serde(default)]
    pub unlocked: bool,
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
            unlock_threshold: 0.6,
            unlocked: true,
        };
        let json = serde_json::to_string(&rec).unwrap();
        let back: CalibrationRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(rec, back);
        assert!(back.unlocked);
        assert_eq!(back.confusion.true_positive, 20);
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
            unlock_threshold: 0.6,
            unlocked: false,
        };
        for set in [
            |r: &mut CalibrationRecord| r.agreement = f64::NAN,
            |r: &mut CalibrationRecord| r.cohen_kappa = f64::INFINITY,
            |r: &mut CalibrationRecord| r.unlock_threshold = f64::NEG_INFINITY,
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
}
