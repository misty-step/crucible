//! Deterministic core for Crucible's code-review eval.
//!
//! The type domains the eval reads, the artifacts it persists, the adapter that
//! bridges two of them, and the deterministic pre-graders:
//!
//! - [`artifact`] — a Cerberus `ReviewArtifact` (the review under evaluation),
//!   mirrored just deeply enough to read findings and their anchors.
//! - [`key`] — Daedalus answer keys in both real shapes: the
//!   `solution/findings.json` point oracle ([`AnswerKey`]) and the
//!   `tests/expected.json` line-span scorer key ([`ExpectedKey`]) that
//!   `daedalus-score` reads — the ground truth a review is scored against.
//! - [`adjudication`] — Crucible's own judgments, keeping correctness
//!   ([`Verdict`]) deliberately distinct from scope ([`Disposition`]).
//! - [`measure`] — uncertainty, agreement, and decision primitives so every
//!   reported rate carries an interval ([`wilson_interval`], [`proportion`]),
//!   judge-vs-human agreement is chance-corrected ([`agreement`],
//!   [`cohen_kappa`]), a paired config delta is refused when it sits inside the
//!   noise floor ([`PairedComparison`], [`DeltaVerdict`]), fixture sets are
//!   sized for the effect they must detect ([`required_sample_size`],
//!   [`power_warning`]), and a derived metric carries a deterministic, seeded
//!   ([`bootstrap_interval`]) interval.
//! - [`adapter`] — projects Cerberus [`Finding`]s onto Daedalus [`KeyFinding`]
//!   rows ([`findings_from_artifact`], [`to_key_findings`]) so a review and a
//!   key can be compared on equal terms.
//! - [`import`] — the opposite direction from [`export`]: projects an
//!   externally-authored eval/benchmark definition onto a Crucible
//!   [`EvalSpec`], total and honest like [`adapter`]. The first supported
//!   format is a Promptfoo-style YAML config ([`parse_promptfoo_config`],
//!   [`project_promptfoo`]) onto the `prompt_benchmark` runner; every test
//!   case that cannot be mapped cleanly is reported in
//!   [`PromptfooImportReport::skipped_tests`], never silently dropped.
//! - [`mod@grade`] — deterministic pre-graders ([`schema_valid`], [`dedup`],
//!   [`key_match`], [`grade`](grade::grade)) that partition a candidate review
//!   against an answer key into matched / disputed / missed before any model or
//!   human judgment. [`recoverable_misses`] re-surfaces the location agreements
//!   the category-strict matcher dropped, so a reported recall is not read as
//!   final.
//! - [`provenance`] — the [`Provenance`] reproducibility kernel and the
//!   top-level [`EvaluationCard`] and [`RunRecord`] that persist it, so a verdict
//!   can be re-run with zero chat context (model, explicit sampling settings,
//!   prompt/rubric hashes, fixtures, cost, caller-supplied timestamp).
//! - [`label`] — an append-only [`Label`]: one human/judge judgment of a single
//!   finding ([`Verdict`] + [`Disposition`]) plus the calibration-validity
//!   conditions it was made under (`latency_ms`, `saw_grader_before_commit`).
//! - [`judgment`] — the adjudication queue: [`build_queue`] turns a
//!   [`GradeResult`]'s disputed candidates (and the misses they could recover)
//!   into an ordered [`JudgmentQueue`], and [`apply_label`] mints the append-only
//!   [`Label`] a judge's verdict produces. A view over the grade, not a third
//!   store; the schema-stamped queue *is* the artifact the phone UI (005) and the
//!   export (002.5) consume.
//! - [`export`] — the write side (002.5): turns a labeled [`JudgmentQueue`] into
//!   the Daedalus answer-key artifacts that *improve the key* — a human
//!   `adjudications.md` log ([`render_adjudications_md`]) in the real
//!   `arenas/<id>/adjudications.md` shape (ACCEPT→key+version bump /
//!   OUT-OF-SCOPE, derived from [`Verdict`]+[`Disposition`]), the
//!   [`extended_key`] `solution/findings.json` oracle, and the
//!   [`extended_expected_key`] `tests/expected.json` scorer key the re-score
//!   actually reads. [`parse_adjudications_md`] inverts the log for Crucible's
//!   own re-reads — that round-trip is internal, not a Daedalus-facing parser.
//! - [`calibration`] — a [`CalibrationRecord`] that *records* (does not compute)
//!   the [`measure`] outputs gating a model/agentic judge: agreement, κ, a
//!   [`ConfusionMatrix`], the unlock threshold, and whether it unlocked.
//! - [`spec`] — the declarative [`EvalSpec`]: task, fixtures by [`FixtureRef`],
//!   a closed-enum [`GraderManifest`] of deterministic / agentic / human graders
//!   ([`GraderKind`]), optional runner/corpus declaration ([`RunnerSpec`],
//!   [`CorpusSpec`]), baselines, aggregation, uncertainty rule, and the
//!   [`Aggregate`] result shape (score + CI + optional paired delta).
//! - [`dashboard`] — the read side of the eval: ingests a tree of real Daedalus
//!   arenas and runs into a [`Dataset`] of [`Eval`] / [`EvalTask`] / [`Config`] /
//!   [`Run`] / [`Trial`]. Each [`Eval`] is one `(arena_id, arena_version)` group,
//!   its identity read from `trials.jsonl` — never the run directory name, which
//!   routinely lies about the arena — and [`Trial`]s pool under a [`Config`] keyed
//!   by `composition_hash` (the stable config identity; `id`/`kind` are mutable
//!   labels). The loader is total: a malformed or unplaceable line is skipped and
//!   counted ([`Dataset::skipped`]), never fatal. This is the opposite direction
//!   from [`export`], which writes Crucible's judgments *back* to Daedalus. The
//!   [`Leaderboard`] then turns that [`Dataset`] into a per-group ranking: a
//!   bootstrap interval ([`Estimate`]) on each config's continuous mean reward, a
//!   Wilson interval on its binary solve rate, and a [`Pairwise`] noise-floor
//!   verdict (McNemar + paired bootstrap) that refuses an indefensible rank gap —
//!   the [`measure`] layer applied to real runs. Its `reward_mean` reconciles with
//!   each run's `summary.json`, so it surfaces Daedalus's own number, not a new one.
//!
//! These types are the narrow waist shared by every later step (matcher,
//! confidence interval). They model only the surface the eval consumes;
//! unrecognized fields in real inputs are ignored, not rejected.
//!
//! The persisted-artifact contract — [`RunRecord`], [`EvaluationCard`], [`Label`],
//! [`CalibrationRecord`], and the declarative [`EvalSpec`] — is the durable waist
//! of backlog 004: each such top-level artifact carries a `schema_version`,
//! defaults its optional fields, and round-trips through serde, so the CLI, the
//! phone adjudication queue (005), and Daedalus all read and write one shared
//! schema. There is no store behind them: the artifacts *are* the API.

mod error;
mod serde_util;

pub mod adapter;
pub mod adjudication;
pub mod artifact;
pub mod calibration;
pub mod dashboard;
pub mod export;
pub mod grade;
pub mod import;
pub mod judgment;
pub mod key;
pub mod label;
pub mod measure;
pub mod provenance;
pub mod spec;

pub use adapter::{findings_from_artifact, to_key_findings};
pub use adjudication::{Disposition, Verdict};
pub use artifact::{Anchor, AnchorKind, Finding, ReviewArtifact, Severity, REVIEW_ARTIFACT_SCHEMA};
pub use calibration::{
    judge_licence_key, model_family, shares_model_family, CalibrationRecord, ConfusionMatrix,
    CALIBRATION_RECORD_SCHEMA,
};
pub use dashboard::{
    Config, Dataset, DeltaEstimate, DeltaSign, Estimate, Eval, EvalTask, Leaderboard,
    LeaderboardEntry, LeaderboardGroup, McnemarOutcome, Pairwise, PairwiseVerdict, Run, SkipReason,
    SkippedInput, Stronger, Trial,
};
pub use error::{Error, Result};
pub use export::{
    adjudications_from_queue, extended_expected_key, extended_key, parse_adjudications_md,
    render_adjudications_md, Adjudication, ArenaVersion, Conditions, ExportContext, ExportError,
    ParsedAdjudications, Ruling,
};
pub use grade::{
    dedup, grade, key_match, location_agrees, recoverable_misses, schema_valid, GradeResult, Match,
    LINE_TOLERANCE,
};
pub use import::{
    parse_promptfoo_config, project_promptfoo, PromptfooAssertion, PromptfooConfig,
    PromptfooImportError, PromptfooImportReport, PromptfooTest, SkippedTest,
};
pub use judgment::{
    apply_label, build_queue, GradeSummary, JudgmentItem, JudgmentQueue, LabelConditions,
    JUDGMENT_QUEUE_SCHEMA,
};
pub use key::{score_against_expected_key, AnswerKey, Defect, ExpectedKey, KeyFinding, SpanGrade};
pub use label::{Label, LABEL_SCHEMA};
pub use measure::{
    agreement, bootstrap_envelope, bootstrap_interval, cohen_kappa, paired_rate_delta_interval,
    power_warning, proportion, required_sample_size, wilson_interval, BootstrapInterval,
    DeltaVerdict, EnsembleInterval, PairedComparison, PairedRateDeltaInterval, PowerWarning,
};
pub use provenance::{
    EvaluationCard, Provenance, RunRecord, RunScore, EVALUATION_CARD_SCHEMA, RUN_RECORD_SCHEMA,
};
pub use spec::{
    AgenticJudgeConfig, AgenticJudgeTask, Aggregate, AggregationMethod, CerberusReceiptTask,
    CorpusSpec, EvalSpec, FixtureRef, Grader, GraderKind, GraderManifest, IntervalMethod,
    ModelProvider, PairedDelta, PromptBenchmarkTask, PromptExpectation, PromptModelConfig,
    RunnerKind, RunnerSpec, UncertaintyRule, EVAL_SPEC_SCHEMA,
};
