//! The declarative eval specification and the shape of its aggregated result.
//!
//! Per backlog 004, a Crucible eval is *data*, not code: one [`EvalSpec`] names
//! the task, its inputs/outputs, the fixtures it runs over (by content
//! [`FixtureRef`]), the [`GraderManifest`] that scores it, the baselines it
//! compares against, how per-item scores aggregate, the uncertainty rule, and
//! the decision the result informs. The same spec drives a near-deterministic
//! eval and a human-judgment-heavy one with no change to core code — the
//! difference lives entirely in the declared grader mix.
//!
//! That grader mix is a closed enum of exactly three [`GraderKind`]s
//! (`deterministic | agentic | human`), deliberately **not** a plugin registry:
//! the runner branches on the kind, so the kind is rigid schema; everything a
//! human or a model reads (task, inputs/outputs, decision) stays free-form text.
//! There is no store, no blob backend, no daemon — the spec and its result
//! *are* the API (backlog 004 non-goals).
//!
//! [`Aggregate`] is the result shape every run emits: a headline score, its
//! confidence interval, and an optional [`PairedDelta`] against a baseline —
//! recording the [`crate::measure`] outputs so no rate is ever reported naked.
//!
//! The CLI can execute narrow, data-shaped spec runners: key-recall over either
//! a Daedalus `trials.jsonl` corpus or freshly produced Cerberus review artifacts,
//! and a prompt benchmark runner that makes Crucible's first live model call.
//! Those runner declarations are deliberately explicit; they name the corpus,
//! model config, and outputs to grade, while the metric and uncertainty still
//! flow through the same [`AggregationMethod`] and [`UncertaintyRule`] fields.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::DeltaVerdict;

/// Schema identifier for a persisted [`EvalSpec`].
pub const EVAL_SPEC_SCHEMA: &str = "crucible.eval_spec.v1";

/// A content-addressed reference to a fixture input, by hash.
///
/// The hash *is* the identity — a digest of the fixture bytes (e.g. a sha256
/// hex), computed by the caller and stored verbatim. Crucible neither hashes nor
/// stores the bytes here: there is no blob backend (backlog 004), so a fixture
/// is *named*, not embedded. Serializes transparently as the bare hash string.
///
/// ```
/// use crucible_core::FixtureRef;
/// let f = FixtureRef("sha256:abc123".to_string());
/// assert_eq!(f.hash(), "sha256:abc123");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureRef(pub String);

impl FixtureRef {
    /// The content hash that identifies the fixture.
    pub fn hash(&self) -> &str {
        &self.0
    }
}

/// One of exactly three grader kinds.
///
/// A closed enum, deliberately **not** a plugin registry (backlog 004): the
/// runner dispatches on the kind, so a fourth kind is a deliberate core change,
/// not a registration. snake_case on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraderKind {
    /// A pure, reproducible check — schema validity, dedup, key-match. No model,
    /// no human, no network.
    Deterministic,
    /// A model / agentic judge, gated behind a [`crate::CalibrationRecord`].
    Agentic,
    /// A human adjudicator — the phone queue (backlog 005).
    Human,
}

/// One named grader in an eval's mix: an id plus which [`GraderKind`] it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grader {
    /// Stable grader id, e.g. `key_match` or `claude-judge`.
    pub id: String,
    /// Which of the three kinds this grader is.
    pub kind: GraderKind,
}

/// The declarative grader mix: the graders that score an eval, in declared
/// order, each one of exactly three [`GraderKind`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraderManifest {
    /// The graders, in the order they run. Defaults to empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graders: Vec<Grader>,
}

impl GraderManifest {
    /// Whether the manifest names no graders.
    pub fn is_empty(&self) -> bool {
        self.graders.is_empty()
    }
}

/// How per-item scores combine into an eval's headline number.
///
/// The two variants map onto the two interval methods the [`crate::measure`]
/// layer ships: a [`Proportion`](Self::Proportion) pairs with a Wilson interval,
/// a [`Mean`](Self::Mean) with a bootstrap interval. snake_case on the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregationMethod {
    /// Fraction of items that passed — a single binomial proportion.
    #[default]
    Proportion,
    /// Arithmetic mean of per-item scores — a derived metric.
    Mean,
}

/// The interval method an eval attaches to its aggregate.
///
/// Mirrors the two [`crate::measure`] primitives: [`Wilson`](Self::Wilson) for a
/// single proportion (small-n safe), [`Bootstrap`](Self::Bootstrap) for a
/// derived/composite metric with no closed-form interval. snake_case on the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalMethod {
    /// Wilson score interval — single binomial proportion.
    #[default]
    Wilson,
    /// Percentile bootstrap — derived/composite metric.
    Bootstrap,
}

/// The rule for attaching an uncertainty interval to an [`Aggregate`].
///
/// Backlog 003 requires every reported rate to carry an interval; this names
/// *which* interval and at what confidence. Defaults to a 95% Wilson interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UncertaintyRule {
    /// The interval method to apply.
    #[serde(default)]
    pub method: IntervalMethod,
    /// Target confidence level in `(0, 1)`, e.g. `0.95`.
    #[serde(
        default = "default_confidence",
        serialize_with = "crate::serde_util::serialize_finite"
    )]
    pub confidence: f64,
}

/// The runner family a declared eval spec can execute.
///
/// This is a closed enum, not an extension registry: adding a new runner means
/// adding a real Crucible-owned execution path. The first runner is the code
/// review key-recall benchmark surface that Threshold/Cerberus can optimize
/// against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerKind {
    /// Score review findings against expected PR-review key rows.
    KeyRecall,
    /// Run authored prompt tasks against a model config and grade the text
    /// response with a deterministic rubric.
    PromptBenchmark,
    /// Grade authored candidate outputs against a rubric with a live model
    /// judge (backlog 012). The `GraderKind::Agentic` tier made real: the spec
    /// must declare an `Agentic` grader in [`EvalSpec::graders`] or the runner
    /// refuses before making a call.
    AgenticJudge,
}

/// One executable runner declaration inside an [`EvalSpec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerSpec {
    /// Which runner family executes this spec.
    pub kind: RunnerKind,
    /// The corpus and candidate output source this runner consumes.
    pub corpus: CorpusSpec,
}

/// One Cerberus-produced review artifact and receipt bundle to grade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CerberusReceiptTask {
    /// Stable task id in the benchmark.
    pub task_id: String,
    /// Cerberus `ReviewArtifact` JSON, absolute or relative to the spec file.
    pub artifact: String,
    /// Cerberus `ReviewReceiptBundle.v1` JSON, absolute or relative to the spec
    /// file. Required so Crucible can distinguish a fresh producer artifact from
    /// an arbitrary JSON fixture.
    pub receipt_bundle: String,
    /// Daedalus Harbor scorer key (`tests/expected.json`), absolute or relative
    /// to the spec file.
    pub expected: String,
}

/// The first provider boundary for Crucible-owned prompt execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelProvider {
    /// OpenAI-compatible Chat Completions through OpenRouter.
    OpenRouter,
}

/// Model config for a prompt benchmark runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptModelConfig {
    /// Provider adapter to use for the live model call.
    pub provider: ModelProvider,
    /// Provider model slug, e.g. `openai/gpt-4o-mini`.
    pub model: String,
    /// System prompt shared by every task in this benchmark.
    pub system_prompt: String,
    /// Environment variable that contains the provider credential.
    #[serde(default = "default_openrouter_credential_env")]
    pub credential_env: String,
    /// Optional output cap for the model call.
    #[serde(
        rename = "max_tokens",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_output_units: Option<u32>,
    /// Optional integer temperature. v0 intentionally supports only whole
    /// values, enough for deterministic `0` without pulling float equality into
    /// the schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<u32>,
}

fn default_openrouter_credential_env() -> String {
    "OPENROUTER_API_KEY".to_string()
}

/// One authored prompt task plus a deterministic rubric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptBenchmarkTask {
    /// Stable task id in the benchmark.
    pub task_id: String,
    /// Optional reporting stratum for class-balanced batteries, e.g.
    /// `code_output` or `long_context_extraction`. Older prompt benchmarks leave
    /// this empty and still deserialize normally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// Optional prompt context file, absolute or relative to the spec file. The
    /// runner prepends its content to `prompt` before the model call. This keeps
    /// long-context fixtures committed as readable files instead of huge escaped
    /// JSON strings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_file: Option<String>,
    /// User prompt for this task.
    pub prompt: String,
    /// Deterministic rubric applied to the model response.
    pub expectation: PromptExpectation,
}

/// Deterministic rubric for prompt benchmarks (backlog 017: the closed-enum
/// grader library, broadened past the original `Exact`/`Contains` pair).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromptExpectation {
    /// The trimmed model response must exactly equal `value`.
    Exact { value: String },
    /// The model response must contain `value`.
    Contains { value: String },
    /// The model response must contain `value`, case-insensitively.
    CaseInsensitiveContains { value: String },
    /// The model response must match the regular expression `pattern`
    /// (unanchored — matches anywhere in the response, per `regex::is_match`).
    /// A pattern that fails to compile is a spec/validation error, not a
    /// grading-time panic; the runner checks this before it makes any model
    /// call.
    Regex { pattern: String },
    /// The trimmed model response must parse as JSON and exactly equal `value`.
    /// This is stricter than text containment: prose around the JSON fails.
    StrictJson { value: Value },
    /// The model response is written to `solution.py` and graded by executing
    /// `test_source` as a committed Python test in an isolated temporary
    /// directory. The runner uses `python3 -I`, clears the environment, and
    /// kills the child after `timeout_ms` (default 3000).
    PythonUnitTest {
        test_source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
}

/// Model config for an agentic judge runner (backlog 012).
///
/// `judge_prompt` is the shared judge framing — the rubric protocol and the
/// verdict format every task's judge call is instructed to follow. Per-task
/// rubric text in [`AgenticJudgeTask::rubric`] is appended per call, not
/// substituted for it: the shared framing is what makes the verdict format
/// (and therefore [`crate::provenance::Provenance::rubric_hash`]) stable
/// across tasks in the same benchmark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgenticJudgeConfig {
    /// Provider adapter to use for the live judge call.
    pub provider: ModelProvider,
    /// Provider model slug for the judge, e.g. `anthropic/claude-opus-4`.
    pub model: String,
    /// Shared judge instructions: framing, grading discipline, and the
    /// required `VERDICT: PASS`/`VERDICT: FAIL` output protocol.
    pub judge_prompt: String,
    /// Environment variable that contains the provider credential.
    #[serde(default = "default_openrouter_credential_env")]
    pub credential_env: String,
    /// Optional integer temperature (see [`PromptModelConfig::temperature`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<u32>,
    /// The model slug that generated the candidate outputs this judge scores,
    /// when known. Enables the self-evaluation bias check (report §6's
    /// self-preference bias table: "judge prefers outputs from same model
    /// family"): the runner compares this against `model`'s
    /// [`crate::model_family`] and records the risk on the
    /// [`crate::CalibrationRecord`] rather than silently allowing it.
    /// `None` when the generator is unrecorded — the calibration record then
    /// reports no generator and no risk, not a false "different family".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_model: Option<String>,
}

/// One candidate output for the judge to score against a rubric.
///
/// A task with `expected_pass: None` is a real candidate: the judge's verdict
/// *is* the score contribution (backlog 012's "judge-gaming guard" needs at
/// least one task with known ground truth to test against, so a corpus of
/// only unknown-truth tasks is real but ungated). A task with
/// `expected_pass: Some(_)` is a calibration probe — most commonly the
/// judge-gaming canary: an obviously-bad candidate with
/// `expected_pass: Some(false)` and `refuse_on_mismatch: true`, wired so a
/// judge that rubber-stamps it refuses the whole run rather than silently
/// shipping an untrustworthy score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgenticJudgeTask {
    /// Stable task id in the benchmark.
    pub task_id: String,
    /// The candidate output the judge scores.
    pub candidate: String,
    /// Task-specific rubric text, appended to the config's shared judge
    /// framing for this call.
    pub rubric: String,
    /// Known ground truth for this task, when it has one. Present only for
    /// calibration probes/canaries; absent for real candidates being judged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_pass: Option<bool>,
    /// When `true` and `expected_pass` is set, a verdict that disagrees with
    /// `expected_pass` refuses the whole run (`anyhow::bail!`s out of the
    /// runner) instead of only counting as a miss. This is the judge-gaming
    /// guard: set it on a canary the judge must reject.
    #[serde(default)]
    pub refuse_on_mismatch: bool,
}

/// The source of examples and candidate outputs for a declared runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum CorpusSpec {
    /// A Daedalus arena directory plus one Daedalus `trials.jsonl` file.
    ///
    /// Each selected trial supplies candidate findings for one task. The runner
    /// grades those findings against
    /// `<arena_dir>/tasks/<task_id>/tests/expected.json`.
    DaedalusTrials {
        /// Daedalus arena directory, absolute or relative to the spec file.
        arena_dir: String,
        /// Daedalus trials JSONL file, absolute or relative to the spec file.
        trials_jsonl: String,
        /// Candidate id to select from the trials file.
        candidate_id: String,
        /// Optional allowlist of task ids. Empty means every trial for the
        /// candidate in the file.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tasks: Vec<String>,
    },
    /// Fresh Cerberus review artifacts handed off with receipt bundles.
    ///
    /// Each task supplies a validated Cerberus artifact plus the Harbor scorer
    /// key Crucible owns. Cerberus remains the producer; Crucible owns the score,
    /// interval, and evidence record.
    CerberusReceiptBundles {
        /// Candidate id to attribute these Cerberus artifacts to.
        candidate_id: String,
        /// Cerberus-produced task outputs to grade.
        tasks: Vec<CerberusReceiptTask>,
    },
    /// Crucible-authored prompt tasks run against a model config.
    PromptBenchmark {
        /// Model provider/config under test.
        config: PromptModelConfig,
        /// Authored prompt tasks to execute and grade.
        tasks: Vec<PromptBenchmarkTask>,
    },
    /// Crucible-authored candidate outputs graded by a live agentic judge
    /// (backlog 012).
    AgenticJudge {
        /// Judge model provider/config under test.
        config: AgenticJudgeConfig,
        /// Candidate outputs and calibration probes to judge.
        tasks: Vec<AgenticJudgeTask>,
    },
}

impl Default for UncertaintyRule {
    fn default() -> Self {
        Self {
            method: IntervalMethod::Wilson,
            confidence: default_confidence(),
        }
    }
}

fn default_confidence() -> f64 {
    0.95
}

/// A declarative evaluation specification: the whole eval as data.
///
/// Names everything needed to run and judge one eval family. `task`, `inputs`,
/// `outputs`, and `decision` are free-form text (read by humans and models);
/// `fixtures`, `graders`, `aggregation`, and `uncertainty` are the rigid schema
/// the runner branches on. Carries a `schema_version` so a persisted spec
/// round-trips across versions; optional fields default so an older or
/// hand-written spec still loads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalSpec {
    /// Schema identifier; defaults to [`EVAL_SPEC_SCHEMA`] for specs that predate
    /// the field. A present value is validated on load — an unknown schema is
    /// rejected, not assumed v1.
    #[serde(
        default = "eval_spec_schema",
        deserialize_with = "deserialize_eval_spec_schema"
    )]
    pub schema_version: String,
    /// Stable eval id, e.g. `pr-review-key-recall-v0`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The task this eval measures, e.g. `code-review`.
    pub task: String,
    /// Free-form description of the inputs the eval consumes. Defaults to empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub inputs: String,
    /// Free-form description of the outputs the eval scores. Defaults to empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outputs: String,
    /// The fixtures this eval runs over, by content hash. Defaults to empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixtures: Vec<FixtureRef>,
    /// The declarative grader mix. Defaults to empty.
    #[serde(default, skip_serializing_if = "GraderManifest::is_empty")]
    pub graders: GraderManifest,
    /// Named baseline configs to compare against, e.g. `known-good`, `known-bad`.
    /// Defaults to empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub baselines: Vec<String>,
    /// How per-item scores aggregate to the headline number.
    #[serde(default)]
    pub aggregation: AggregationMethod,
    /// The rule for attaching uncertainty to the aggregate.
    #[serde(default)]
    pub uncertainty: UncertaintyRule,
    /// The decision this eval informs, in one human sentence. Defaults to empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub decision: String,
    /// Executable runner declaration. Omitted specs are definition-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<RunnerSpec>,
}

fn eval_spec_schema() -> String {
    EVAL_SPEC_SCHEMA.to_string()
}

fn deserialize_eval_spec_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, EVAL_SPEC_SCHEMA)
}

/// A paired-configuration delta against a baseline, recorded only when one was
/// computed.
///
/// Records a [`crate::PairedComparison`] outcome for persistence: the point delta
/// on the shared metric, the McNemar two-sided p-value, and the [`DeltaVerdict`]
/// that says whether the delta cleared the noise floor. Storing the verdict (not
/// just the p-value) keeps "refuse to report a delta you cannot defend" legible
/// in the artifact itself.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PairedDelta {
    /// The point estimate of the delta (B − A) on the shared metric.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub delta: f64,
    /// McNemar two-sided p-value for the paired comparison.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub p_value: f64,
    /// Whether the delta is signal or sits inside the noise floor.
    pub verdict: DeltaVerdict,
}

/// The aggregated result of an eval run: a score that never travels without its
/// uncertainty.
///
/// Distinct from [`AggregationMethod`] (the *method* a spec declares); this is
/// the computed *result*. Backlog 003: every score ships with an interval, and a
/// [`PairedDelta`] is attached only when a paired comparison was run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aggregate {
    /// The headline score (a pass rate, a mean, …).
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub score: f64,
    /// The confidence interval `(lower, upper)` around `score`.
    #[serde(serialize_with = "crate::serde_util::serialize_finite_pair")]
    pub ci: (f64, f64),
    /// The paired delta against a baseline, when one was computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_delta: Option<PairedDelta>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_ref_serializes_as_bare_hash_string() {
        let f = FixtureRef("sha256:abc123".to_string());
        assert_eq!(serde_json::to_string(&f).unwrap(), "\"sha256:abc123\"");
        let back: FixtureRef = serde_json::from_str("\"sha256:abc123\"").unwrap();
        assert_eq!(f, back);
        assert_eq!(back.hash(), "sha256:abc123");
    }

    #[test]
    fn grader_kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&GraderKind::Deterministic).unwrap(),
            "\"deterministic\""
        );
        assert_eq!(
            serde_json::to_string(&GraderKind::Agentic).unwrap(),
            "\"agentic\""
        );
        assert_eq!(
            serde_json::to_string(&GraderKind::Human).unwrap(),
            "\"human\""
        );
        let k: GraderKind = serde_json::from_str("\"human\"").unwrap();
        assert_eq!(k, GraderKind::Human);
    }

    #[test]
    fn aggregation_and_interval_methods_default() {
        assert_eq!(AggregationMethod::default(), AggregationMethod::Proportion);
        assert_eq!(IntervalMethod::default(), IntervalMethod::Wilson);
    }

    #[test]
    fn uncertainty_rule_defaults_to_wilson_95() {
        let u = UncertaintyRule::default();
        assert_eq!(u.method, IntervalMethod::Wilson);
        assert_eq!(u.confidence, 0.95);
        // An empty object fills both field defaults, not f64's 0.0.
        let from_empty: UncertaintyRule = serde_json::from_str("{}").unwrap();
        assert_eq!(from_empty, u);
    }

    #[test]
    fn minimal_spec_serializes_to_golden_and_round_trips() {
        let spec = EvalSpec {
            schema_version: EVAL_SPEC_SCHEMA.to_string(),
            id: String::new(),
            task: "code-review".to_string(),
            inputs: String::new(),
            outputs: String::new(),
            fixtures: Vec::new(),
            graders: GraderManifest::default(),
            baselines: Vec::new(),
            aggregation: AggregationMethod::Proportion,
            uncertainty: UncertaintyRule::default(),
            decision: String::new(),
            runner: None,
        };
        // Every empty optional is skipped; only the required + non-empty
        // structured fields remain.
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            json,
            r#"{"schema_version":"crucible.eval_spec.v1","task":"code-review","aggregation":"proportion","uncertainty":{"method":"wilson","confidence":0.95}}"#
        );
        let back: EvalSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn bare_spec_fills_all_defaults() {
        // A spec naming only its task must load, defaulting schema, aggregation,
        // uncertainty, and every collection.
        let spec: EvalSpec = serde_json::from_str(r#"{"task":"code-review"}"#).unwrap();
        assert_eq!(spec.schema_version, EVAL_SPEC_SCHEMA);
        assert!(spec.id.is_empty());
        assert_eq!(spec.task, "code-review");
        assert_eq!(spec.aggregation, AggregationMethod::Proportion);
        assert_eq!(spec.uncertainty, UncertaintyRule::default());
        assert!(spec.fixtures.is_empty());
        assert!(spec.graders.is_empty());
        assert!(spec.baselines.is_empty());
        assert!(spec.runner.is_none());
    }

    #[test]
    fn full_spec_round_trips_with_mixed_graders() {
        let spec = EvalSpec {
            schema_version: EVAL_SPEC_SCHEMA.to_string(),
            id: "code-review-calibration-v0".to_string(),
            task: "code-review".to_string(),
            inputs: "Cerberus ReviewArtifact over a diff".to_string(),
            outputs: "matched / disputed / missed".to_string(),
            fixtures: vec![
                FixtureRef("sha256:aa".to_string()),
                FixtureRef("sha256:bb".to_string()),
            ],
            graders: GraderManifest {
                graders: vec![
                    Grader {
                        id: "key_match".to_string(),
                        kind: GraderKind::Deterministic,
                    },
                    Grader {
                        id: "claude-judge".to_string(),
                        kind: GraderKind::Agentic,
                    },
                    Grader {
                        id: "operator".to_string(),
                        kind: GraderKind::Human,
                    },
                ],
            },
            baselines: vec!["known-good".to_string(), "known-bad".to_string()],
            aggregation: AggregationMethod::Mean,
            uncertainty: UncertaintyRule {
                method: IntervalMethod::Bootstrap,
                confidence: 0.9,
            },
            decision: "ship the config with the higher calibrated keep-rate".to_string(),
            runner: Some(RunnerSpec {
                kind: RunnerKind::KeyRecall,
                corpus: CorpusSpec::DaedalusTrials {
                    arena_dir: "../daedalus/arenas/pr-review-v0".to_string(),
                    trials_jsonl: "../daedalus/runs/freeze/trials.jsonl".to_string(),
                    candidate_id: "probe-oneshot".to_string(),
                    tasks: vec!["py-file-cache".to_string()],
                },
            }),
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: EvalSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
        assert_eq!(back.graders.graders.len(), 3);
        assert_eq!(back.graders.graders[1].kind, GraderKind::Agentic);
        assert_eq!(back.runner.unwrap().kind, RunnerKind::KeyRecall);
    }

    #[test]
    fn cerberus_receipt_bundle_corpus_round_trips() {
        let corpus = CorpusSpec::CerberusReceiptBundles {
            candidate_id: "cerberus-live".to_string(),
            tasks: vec![CerberusReceiptTask {
                task_id: "ratio-zero".to_string(),
                artifact: "../runs/ratio-zero/artifact.json".to_string(),
                receipt_bundle: "../runs/ratio-zero/receipt-bundle.json".to_string(),
                expected:
                    "../../daedalus/arenas/cerberus-fixture-v0/tasks/ratio-zero/tests/expected.json"
                        .to_string(),
            }],
        };
        let json = serde_json::to_string(&corpus).unwrap();
        assert!(
            json.contains(r#""source":"cerberus_receipt_bundles""#),
            "corpus source is stable: {json}"
        );
        let back: CorpusSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, corpus);
    }

    #[test]
    fn prompt_benchmark_corpus_round_trips() {
        let corpus = CorpusSpec::PromptBenchmark {
            config: PromptModelConfig {
                provider: ModelProvider::OpenRouter,
                model: "openai/gpt-4o-mini".to_string(),
                system_prompt: "Answer exactly.".to_string(),
                credential_env: "OPENROUTER_API_KEY".to_string(),
                max_output_units: Some(8),
                temperature: Some(0),
            },
            tasks: vec![PromptBenchmarkTask {
                task_id: "exact-word".to_string(),
                class: Some("format_adherence".to_string()),
                context_file: None,
                prompt: "Reply with exactly: crucible-smoke".to_string(),
                expectation: PromptExpectation::Exact {
                    value: "crucible-smoke".to_string(),
                },
            }],
        };
        let json = serde_json::to_string(&corpus).unwrap();
        assert!(
            json.contains(r#""source":"prompt_benchmark""#),
            "corpus source is stable: {json}"
        );
        let back: CorpusSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, corpus);
    }

    #[test]
    fn aggregate_omits_paired_delta_when_absent() {
        let agg = Aggregate {
            score: 0.8,
            ci: (0.49, 0.94),
            paired_delta: None,
        };
        let json = serde_json::to_string(&agg).unwrap();
        // The CI is a JSON array; the absent paired delta is skipped entirely.
        assert_eq!(json, r#"{"score":0.8,"ci":[0.49,0.94]}"#);
        let back: Aggregate = serde_json::from_str(&json).unwrap();
        assert_eq!(agg, back);
    }

    #[test]
    fn aggregate_records_paired_delta_verdict() {
        let agg = Aggregate {
            score: 0.8,
            ci: (0.49, 0.94),
            paired_delta: Some(PairedDelta {
                delta: 0.1,
                p_value: 0.0215,
                verdict: DeltaVerdict::Signal,
            }),
        };
        let json = serde_json::to_string(&agg).unwrap();
        assert!(
            json.contains(r#""verdict":"signal""#),
            "verdict not recorded: {json}"
        );
        let back: Aggregate = serde_json::from_str(&json).unwrap();
        assert_eq!(agg, back);
        assert_eq!(back.paired_delta.unwrap().verdict, DeltaVerdict::Signal);
    }

    #[test]
    fn non_finite_score_or_interval_is_refused() {
        // A non-finite score or CI bound would serialize to a JSON null that
        // fails to read back as f64; serialization must error instead.
        let base = Aggregate {
            score: 0.8,
            ci: (0.49, 0.94),
            paired_delta: None,
        };
        let mut bad_score = base;
        bad_score.score = f64::NAN;
        assert!(
            serde_json::to_string(&bad_score).is_err(),
            "a NaN score must not serialize to a non-round-tripping null"
        );
        let mut bad_ci = base;
        bad_ci.ci = (0.49, f64::INFINITY);
        assert!(
            serde_json::to_string(&bad_ci).is_err(),
            "a non-finite CI bound must not serialize"
        );
        // A non-finite paired-delta p-value is refused too.
        let bad_delta = Aggregate {
            score: 0.8,
            ci: (0.49, 0.94),
            paired_delta: Some(PairedDelta {
                delta: 0.1,
                p_value: f64::NAN,
                verdict: DeltaVerdict::Signal,
            }),
        };
        assert!(serde_json::to_string(&bad_delta).is_err());
    }

    #[test]
    fn unknown_spec_schema_version_is_rejected() {
        let json = r#"{"schema_version":"crucible.eval_spec.v999","task":"code-review"}"#;
        let err = serde_json::from_str::<EvalSpec>(json).unwrap_err();
        assert!(
            err.to_string().contains("schema_version"),
            "error should name the bad schema_version: {err}"
        );
    }
}
