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
//! `crucible run` on an `agentic_judge` spec writes one record per run
//! (`build_calibration_record` in `crucible/src/spec_run.rs`); ground truth is
//! normally the spec author's declared `expected_pass`, though
//! [`expected_verdicts_from_labels`] lets a caller source (or supplement) it
//! from collected human [`crate::Label`] judgments instead, via a documented
//! Keep/Nit→pass, Wrong/Noise→fail mapping. Backlog 970 (v2) adds: fail-class
//! precision/recall (a bare Cohen's κ hides minority-class blindness), a
//! `task_family` axis folded into [`judge_licence_key`] so a licence earned on
//! one task family never silently covers another, and an opt-in cross-run
//! drift check ([`probe_drift`]) distinct from the within-run
//! format-sensitivity self-check.
//!
//! The gate this record measures is structural, not a note string (backlog
//! 971): `unlocked` (or its absence) is projected onto every persisted run as
//! `run_records.trusted` in the SQLite ledger, and
//! `crucible::run_store::compare_configs` refuses — `comparison_kind:
//! "untrusted_run_refused"`, `paired`/`resolution` left `None` — any
//! comparison naming an untrusted run. Since the findings journal derives
//! every finding from a comparison's `paired` field, a locked judge's score
//! cannot produce a `crucible.finding.v1` Signal record, not merely one that
//! carries a "diagnostic" note a reader could ignore.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Label, Verdict};

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
/// judge system prompt, calibration rubric set, **task family**). This is what
/// makes calibration state "invalidated when judge model/prompt/rubric/family
/// changes" mechanical rather than a separate check to remember: any change to
/// one of the four inputs yields a different key, so looking up the new key
/// simply finds no prior licence — locked/unlicensed until a run under the new
/// key establishes one. See [`CalibrationRecord::licence_key`].
///
/// The `v2` prefix (bumped from backlog 970's predecessor, which had no
/// `task_family` segment) is deliberate: an old key can never collide with a
/// new one even if a caller mistakenly reused a `v1`-shaped lookup, so
/// cross-family (and cross-version) reuse of a calibration is structurally
/// impossible rather than merely discouraged (backlog 970's "why": a judge
/// trusted on code-review must not be silently trusted on a new task family).
pub fn judge_licence_key(
    judge_model: &str,
    system_prompt_hash: &str,
    rubric_hash: &str,
    task_family: &str,
) -> String {
    format!("judge-licence:v2:{judge_model}:{system_prompt_hash}:{rubric_hash}:{task_family}")
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

    /// Precision of the judge's **fail** (negative) calls: of the cases the
    /// judge called fail, the fraction the human reference also called fail.
    /// This is the minority-class metric a bare Cohen's κ hides (Hamel;
    /// Yan: 80-87% raw agreement collapsing to κ 0.3-0.5 under class
    /// imbalance) — derived from the existing counts, no new measurement.
    /// `0.0` when the judge never called fail (never `NaN`).
    pub fn fail_precision(&self) -> f64 {
        safe_rate(self.true_negative, self.true_negative + self.false_negative)
    }

    /// Recall of the judge's **fail** (negative) calls: of the cases the human
    /// reference marked fail, the fraction the judge also called fail. `0.0`
    /// when there are no actual-fail cases (never `NaN`).
    pub fn fail_recall(&self) -> f64 {
        safe_rate(self.true_negative, self.true_negative + self.false_positive)
    }
}

/// Compare a judge's calibration verdicts across two runs of the *same* probe
/// set — the cross-run drift check backlog 970 asks for: "the same judge+prompt
/// re-run on a different day swings 8-15%" (Scale AI), a fragility no single
/// calibration run can see on its own. Distinct from
/// [`CalibrationRecord::format_sensitivity_flip_rate`] (a within-run cosmetic
/// perturbation self-check) — this measures the identical call repeated across
/// sessions.
///
/// Matches `current` against `baseline` by task id and reports the fraction of
/// the *shared* tasks whose verdict flipped, plus how many tasks were shared.
/// `None` when there is no overlap — a probe set with zero shared task ids
/// cannot report a rate about "the same call repeated," so this refuses to
/// fabricate a `0.0` over an empty or wholly disjoint intersection.
pub fn probe_drift(
    baseline: &BTreeMap<String, bool>,
    current: &BTreeMap<String, bool>,
) -> Option<(f64, u64)> {
    let mut n = 0u64;
    let mut flips = 0u64;
    for (task_id, current_verdict) in current {
        if let Some(baseline_verdict) = baseline.get(task_id) {
            n += 1;
            if baseline_verdict != current_verdict {
                flips += 1;
            }
        }
    }
    if n == 0 {
        return None;
    }
    Some((flips as f64 / n as f64, n))
}

/// The documented mapping from a human adjudication [`Verdict`] to a
/// calibration-style pass/fail boolean (backlog 970): `Keep`/`Nit` are
/// "pass-ish" (the finding was judged correct, whether or not it was trivial)
/// and `Wrong`/`Noise` are "fail-ish" (the finding was judged incorrect or not
/// real). This mapping is a policy choice, not a derivation — it is named here,
/// once, so every caller sourcing calibration ground truth from human labels
/// uses the same rule rather than inventing its own.
pub fn label_calibration_verdict(verdict: Verdict) -> bool {
    matches!(verdict, Verdict::Keep | Verdict::Nit)
}

/// Project collected human [`Label`]s into calibration ground truth: `(finding_id,
/// expected_pass)` pairs a caller pairs against a judge's own verdict on the
/// same finding to build a [`CalibrationRecord`] sourced from human judgment
/// rather than (or alongside) a spec author's declared `expected_pass`.
///
/// Labels with `saw_grader_before_commit: true` are excluded: per
/// [`crate::label`]'s own contract, a judgment made after the grader's verdict
/// was revealed is not valid *blind* calibration data — including it here would
/// let a judge's own answer contaminate the ground truth measuring it.
pub fn expected_verdicts_from_labels(labels: &[Label]) -> Vec<(String, bool)> {
    labels
        .iter()
        .filter(|label| !label.saw_grader_before_commit)
        .map(|label| {
            (
                label.finding_id.clone(),
                label_calibration_verdict(label.verdict),
            )
        })
        .collect()
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
    /// Fraction of the decisive calibration verdicts that flipped when the
    /// same rubric/candidate pair was re-judged with a purely cosmetic
    /// prompt perturbation (rubric/candidate section order swapped) — the
    /// format-sensitivity self-check (*Evaluating Scoring Bias in
    /// LLM-as-a-Judge*, arXiv:2506.22316). `None` when the check was not run
    /// (opt-in via [`crate::AgenticJudgeConfig::format_sensitivity_check`]),
    /// distinct from `Some(0.0)` (checked, and stable).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::serde_util::serialize_finite_option"
    )]
    pub format_sensitivity_flip_rate: Option<f64>,
    /// Number of calibration items the format-sensitivity check re-judged to
    /// produce [`Self::format_sensitivity_flip_rate`]. `0` when the check was
    /// not run or had no decisive calibration items to sample.
    #[serde(default)]
    pub format_sensitivity_n: u64,
    /// Precision of the judge's fail calls against the human/deterministic
    /// reference ([`ConfusionMatrix::fail_precision`]) — the minority-class
    /// metric a bare [`Self::cohen_kappa`] hides. Defaults to `0.0`.
    #[serde(default, serialize_with = "crate::serde_util::serialize_finite")]
    pub fail_class_precision: f64,
    /// Recall of the judge's fail calls ([`ConfusionMatrix::fail_recall`]).
    /// Defaults to `0.0`.
    #[serde(default, serialize_with = "crate::serde_util::serialize_finite")]
    pub fail_class_recall: f64,
    /// The task family this calibration is scoped to (e.g. [`crate::spec::EvalSpec::task`]),
    /// folded into [`judge_licence_key`] so a licence earned on one family
    /// cannot silently cover another. Defaults to empty for records that
    /// predate this field (backlog 970) — an empty family matches no
    /// family-scoped licence lookup, the safe (locked/unknown) direction.
    #[serde(default)]
    pub task_family: String,
    /// Fraction of the shared calibration probe tasks whose judge verdict
    /// flipped versus a prior run over the same probe set — [`probe_drift`]'s
    /// output, the cross-run/cross-session drift check. `None` when no prior
    /// run was supplied for comparison, distinct from `Some(0.0)` (compared,
    /// and stable). Distinct from [`Self::format_sensitivity_flip_rate`] (a
    /// within-run cosmetic-perturbation self-check, not a repeated identical
    /// call).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::serde_util::serialize_finite_option"
    )]
    pub drift_flip_rate: Option<f64>,
    /// Number of shared probe tasks [`probe_drift`] matched between the two
    /// runs. `0` when the check was not run or the two probe sets shared no
    /// task ids.
    #[serde(default)]
    pub drift_probe_n: u64,
    /// Caller-supplied Unix-seconds timestamp of when the drift check ran.
    /// `None` when no drift check was performed. Nothing in this module reads
    /// the clock, mirroring [`crate::Label::timestamp`]'s caller-supplied
    /// discipline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift_checked_at: Option<i64>,
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
            licence_key: "judge-licence:v2:claude-judge:hash1:hash2:code-review".to_string(),
            format_sensitivity_flip_rate: Some(0.1),
            format_sensitivity_n: 10,
            fail_class_precision: 0.85,
            fail_class_recall: 0.85,
            task_family: "code-review".to_string(),
            drift_flip_rate: Some(0.05),
            drift_probe_n: 20,
            drift_checked_at: Some(1_783_000_000),
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
            "judge-licence:v2:claude-judge:hash1:hash2:code-review"
        );
        assert_eq!(back.format_sensitivity_flip_rate, Some(0.1));
        assert_eq!(back.format_sensitivity_n, 10);
        assert_eq!(back.fail_class_precision, 0.85);
        assert_eq!(back.fail_class_recall, 0.85);
        assert_eq!(back.task_family, "code-review");
        assert_eq!(back.drift_flip_rate, Some(0.05));
        assert_eq!(back.drift_probe_n, 20);
        assert_eq!(back.drift_checked_at, Some(1_783_000_000));
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
        assert_eq!(
            rec.format_sensitivity_flip_rate, None,
            "an omitted format-sensitivity flip rate defaults to None (not run), not Some(0.0)"
        );
        assert_eq!(rec.format_sensitivity_n, 0);
        assert_eq!(
            rec.fail_class_precision, 0.0,
            "an omitted fail-class precision defaults to 0.0"
        );
        assert_eq!(
            rec.fail_class_recall, 0.0,
            "an omitted fail-class recall defaults to 0.0"
        );
        assert_eq!(
            rec.task_family, "",
            "an omitted task family defaults to empty — matches no family-scoped lookup"
        );
        assert_eq!(
            rec.drift_flip_rate, None,
            "an omitted drift flip rate defaults to None (not checked), not Some(0.0)"
        );
        assert_eq!(rec.drift_probe_n, 0);
        assert_eq!(rec.drift_checked_at, None);
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
            licence_key: "judge-licence:v2:claude-judge:hash1:hash2:code-review".to_string(),
            format_sensitivity_flip_rate: Some(0.0),
            format_sensitivity_n: 4,
            fail_class_precision: 0.9,
            fail_class_recall: 0.9,
            task_family: "code-review".to_string(),
            drift_flip_rate: Some(0.0),
            drift_probe_n: 4,
            drift_checked_at: Some(1_783_000_000),
        };
        for set in [
            |r: &mut CalibrationRecord| r.agreement = f64::NAN,
            |r: &mut CalibrationRecord| r.cohen_kappa = f64::INFINITY,
            |r: &mut CalibrationRecord| r.unlock_threshold = f64::NEG_INFINITY,
            |r: &mut CalibrationRecord| r.false_positive_rate = f64::NAN,
            |r: &mut CalibrationRecord| r.false_negative_rate = f64::INFINITY,
            |r: &mut CalibrationRecord| r.format_sensitivity_flip_rate = Some(f64::NAN),
            |r: &mut CalibrationRecord| r.fail_class_precision = f64::NAN,
            |r: &mut CalibrationRecord| r.fail_class_recall = f64::INFINITY,
            |r: &mut CalibrationRecord| r.drift_flip_rate = Some(f64::NAN),
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
    fn judge_licence_key_changes_with_any_of_its_four_inputs() {
        let base = judge_licence_key(
            "openai/gpt-4o",
            "prompt-hash-1",
            "rubric-hash-1",
            "code-review",
        );
        assert_ne!(
            base,
            judge_licence_key(
                "anthropic/claude-opus-4",
                "prompt-hash-1",
                "rubric-hash-1",
                "code-review"
            ),
            "a different judge model must yield a different licence key"
        );
        assert_ne!(
            base,
            judge_licence_key(
                "openai/gpt-4o",
                "prompt-hash-2",
                "rubric-hash-1",
                "code-review"
            ),
            "a different judge prompt must yield a different licence key"
        );
        assert_ne!(
            base,
            judge_licence_key(
                "openai/gpt-4o",
                "prompt-hash-1",
                "rubric-hash-2",
                "code-review"
            ),
            "a different rubric set must yield a different licence key"
        );
        assert_ne!(
            base,
            judge_licence_key(
                "openai/gpt-4o",
                "prompt-hash-1",
                "rubric-hash-1",
                "doc-review"
            ),
            "a different task family must yield a different licence key — cross-family reuse is structurally impossible"
        );
        assert_eq!(
            base,
            judge_licence_key(
                "openai/gpt-4o",
                "prompt-hash-1",
                "rubric-hash-1",
                "code-review"
            ),
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

    // ---- fail-class precision/recall -----------------------------------------

    #[test]
    fn fail_class_precision_and_recall_are_the_negative_class_metrics() {
        let confusion = ConfusionMatrix {
            true_positive: 20,
            false_positive: 4,
            false_negative: 4,
            true_negative: 22,
        };
        // Of the 26 judge-called-fail cases (4 FN + 22 TN), 22 were actually fail.
        assert!((confusion.fail_precision() - (22.0 / 26.0)).abs() < 1e-9);
        // Of the 26 actually-fail cases (4 FP + 22 TN), 22 were called fail.
        assert!((confusion.fail_recall() - (22.0 / 26.0)).abs() < 1e-9);
    }

    #[test]
    fn fail_class_metrics_are_zero_not_nan_when_the_judge_never_calls_fail() {
        // TN=0, FN=0: the judge never called fail, so fail-class precision has
        // no denominator to divide by (never NaN).
        let confusion = ConfusionMatrix {
            true_positive: 10,
            false_positive: 0,
            false_negative: 0,
            true_negative: 0,
        };
        assert_eq!(confusion.fail_precision(), 0.0);
        assert_eq!(confusion.fail_recall(), 0.0);
    }

    // ---- drift check -----------------------------------------------------------

    #[test]
    fn probe_drift_reports_the_flip_rate_over_shared_task_ids() {
        let baseline = BTreeMap::from([
            ("t1".to_string(), true),
            ("t2".to_string(), false),
            ("t3".to_string(), true),
            ("t4".to_string(), true),
        ]);
        let current = BTreeMap::from([
            ("t1".to_string(), true),  // stable
            ("t2".to_string(), true),  // flipped
            ("t3".to_string(), false), // flipped
            ("t4".to_string(), true),  // stable
        ]);
        let (rate, n) = probe_drift(&baseline, &current).expect("all four tasks overlap");
        assert_eq!(n, 4);
        assert!((rate - 0.5).abs() < 1e-9, "2 of 4 flipped: {rate}");
    }

    #[test]
    fn probe_drift_only_counts_shared_task_ids() {
        let baseline = BTreeMap::from([("t1".to_string(), true), ("t2".to_string(), false)]);
        let current = BTreeMap::from([("t2".to_string(), true), ("t3".to_string(), true)]);
        // Only t2 is shared, and it flipped.
        let (rate, n) = probe_drift(&baseline, &current).expect("t2 overlaps");
        assert_eq!(n, 1);
        assert_eq!(rate, 1.0);
    }

    #[test]
    fn probe_drift_is_none_when_the_probe_sets_share_no_task_ids() {
        let baseline = BTreeMap::from([("t1".to_string(), true)]);
        let current = BTreeMap::from([("t2".to_string(), true)]);
        assert_eq!(
            probe_drift(&baseline, &current),
            None,
            "zero overlap cannot report a rate about 'the same call repeated'"
        );
    }

    // ---- human-label calibration sourcing --------------------------------------

    fn label(finding_id: &str, verdict: Verdict, saw_grader_before_commit: bool) -> Label {
        Label {
            schema_version: crate::label::LABEL_SCHEMA.to_string(),
            finding_id: finding_id.to_string(),
            verdict,
            disposition: crate::Disposition { in_scope: true },
            latency_ms: 0,
            saw_grader_before_commit,
            timestamp: String::new(),
        }
    }

    #[test]
    fn label_calibration_verdict_maps_keep_and_nit_to_pass_wrong_and_noise_to_fail() {
        assert!(label_calibration_verdict(Verdict::Keep));
        assert!(label_calibration_verdict(Verdict::Nit));
        assert!(!label_calibration_verdict(Verdict::Wrong));
        assert!(!label_calibration_verdict(Verdict::Noise));
    }

    #[test]
    fn expected_verdicts_from_labels_maps_blind_labels_only() {
        let labels = vec![
            label("F1", Verdict::Keep, false),
            label("F2", Verdict::Wrong, false),
            // Revealed (saw the grader's verdict first) — not valid blind
            // calibration data, must be excluded.
            label("F3", Verdict::Keep, true),
        ];
        let expected = expected_verdicts_from_labels(&labels);
        assert_eq!(
            expected,
            vec![("F1".to_string(), true), ("F2".to_string(), false),],
            "F3 is excluded: it was labeled after the grader's verdict was revealed"
        );
    }
}
