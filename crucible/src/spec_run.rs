//! Declared `EvalSpec` execution for the first real benchmark surface.
//!
//! This is intentionally narrower than a general runner framework. It executes
//! the first load-bearing spec shape Crucible needs now: key recall over
//! Daedalus PR-review `trials.jsonl` corpora and fresh Cerberus review artifacts
//! handed off with receipt bundles. New runner families should earn their own
//! explicit branch in the spec schema and here.

use std::collections::{BTreeMap, HashSet};
use std::io::BufRead;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use crucible_core::{
    agreement, cohen_kappa, findings_from_artifact, judge_licence_key, probe_drift, schema_valid,
    shares_model_family, to_key_findings, AgenticJudgeConfig, AgenticJudgeTask, AggregationMethod,
    CalibrationRecord, CerberusReceiptTask, ConfusionMatrix, CorpusSpec, EvalSpec, ExpectedKey,
    GraderKind, HarborRunConfig, HarborTaskSpec, IntervalMethod, KeyFinding, ModelProvider,
    PromptBenchmarkTask, PromptExpectation, PromptModelConfig, ResourceEnvelope, RunnerKind,
    RunnerSpec, Trace, TraceStep, CALIBRATION_RECORD_SCHEMA, TRACE_SCHEMA,
};
use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::eval_run::{EvalReport, RunReport, Score, RUN_REPORT_SCHEMA};
use crate::wilson_score;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RunOptions {
    pub prompt_model: Option<String>,
    pub prompt_system_prompt: Option<String>,
    pub prompt_max_output_units: Option<u32>,
    pub prompt_temperature: Option<u32>,
}

impl RunOptions {
    pub(crate) fn with_prompt_model(model: impl Into<String>) -> Self {
        Self {
            prompt_model: Some(model.into()),
            ..Self::default()
        }
    }
}

/// Execute a declared eval spec and write a `crucible.run_report.v1` plus
/// runner-specific evidence under `out`.
pub fn run(spec_path: &Path, out: Option<&Path>) -> anyhow::Result<RunReport> {
    run_with_options(spec_path, out, &RunOptions::default())
}

pub fn run_with_options(
    spec_path: &Path,
    out: Option<&Path>,
    options: &RunOptions,
) -> anyhow::Result<RunReport> {
    let spec = load_spec(spec_path)?;
    run_loaded_spec(&spec, spec_path, out, options)
}

/// Execute an already-loaded [`EvalSpec`] value, rather than reading it from
/// disk. `spec_path` is still required — it anchors relative fixture/corpus
/// paths and supplies the spec-id fallback — but the spec that actually runs is
/// the `spec` argument, not the file's contents. This is the entry point the
/// environment matrix (`crucible run --env`) uses to run a spec transformed by
/// an [`crucible_core::Environment`] without writing the transformed spec back
/// to disk.
pub(crate) fn run_loaded_spec(
    spec: &EvalSpec,
    spec_path: &Path,
    out: Option<&Path>,
    options: &RunOptions,
) -> anyhow::Result<RunReport> {
    let out = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_output_dir_for_spec(spec, spec_path));
    std::fs::create_dir_all(&out)
        .with_context(|| format!("creating run output directory {}", out.display()))?;

    let runner = spec.runner.as_ref().with_context(|| {
        format!(
            "spec {} is definition-only: it has no executable runner declaration",
            spec_path.display()
        )
    })?;
    let eval = run_runner(spec, runner, spec_path, &out, options)?;
    let report = RunReport {
        schema_version: RUN_REPORT_SCHEMA,
        output_dir: out.display().to_string(),
        evals: vec![eval],
    };
    write_json(&out.join("run-report.json"), &report)?;
    Ok(report)
}

pub(crate) fn default_output_dir(spec_path: &Path) -> anyhow::Result<PathBuf> {
    let spec = load_spec(spec_path)?;
    Ok(default_output_dir_for_spec(&spec, spec_path))
}

pub(crate) fn load_spec(spec_path: &Path) -> anyhow::Result<EvalSpec> {
    let bytes = std::fs::read(spec_path)
        .with_context(|| format!("reading eval spec {}", spec_path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {} as a Crucible EvalSpec", spec_path.display()))
}

fn default_output_dir_for_spec(spec: &EvalSpec, spec_path: &Path) -> PathBuf {
    Path::new("runs/local").join(spec_id(spec, spec_path))
}

/// The grader kind a runner requires declared in `graders.graders` before it
/// will execute — the grading tier the runner actually performs, not an
/// aspiration. `key_recall`/`prompt_benchmark` grade deterministically;
/// `agentic_judge` makes the live judge call `GraderKind::Agentic` names.
pub(crate) fn required_grader_kind(runner_kind: RunnerKind) -> GraderKind {
    match runner_kind {
        RunnerKind::KeyRecall | RunnerKind::PromptBenchmark | RunnerKind::HarborTask => {
            GraderKind::Deterministic
        }
        RunnerKind::AgenticJudge => GraderKind::Agentic,
    }
}

/// The checks every runner enforces before it will make a call or read a
/// corpus — shared by the real bail path below and `crucible validate`
/// (backlog 014), so the two can never drift apart. Refuses (does not merely
/// warn) a spec that declares a runner kind's aggregation, uncertainty
/// method/confidence, or grader mix it cannot honor: backlog 014's "wired or
/// removed," applied to `aggregation`, `uncertainty.confidence`, and
/// `graders` (`fixtures` already flows into evaluation-card provenance;
/// `baselines` remains genuinely unenforced — not claimed honest here).
pub(crate) fn preflight_spec(spec: &EvalSpec, runner_kind: RunnerKind) -> anyhow::Result<()> {
    let label = runner_kind_label(runner_kind);
    if spec.aggregation != AggregationMethod::Proportion {
        anyhow::bail!(
            "{label} runner requires aggregation=proportion, got {:?}",
            spec.aggregation
        );
    }
    if spec.uncertainty.method != IntervalMethod::Wilson {
        anyhow::bail!(
            "{label} runner requires uncertainty.method=wilson, got {:?}",
            spec.uncertainty.method
        );
    }
    // The runner always computes a Wilson interval at 95% confidence
    // (main.rs's Z_95 = 1.96, hardcoded) regardless of what a spec declares.
    // A spec declaring anything else is a lie the runner used to tell by
    // silently ignoring the field; refuse instead.
    const HONORED_CONFIDENCE: f64 = 0.95;
    if (spec.uncertainty.confidence - HONORED_CONFIDENCE).abs() > f64::EPSILON {
        anyhow::bail!(
            "{label} runner only computes a {:.2} confidence Wilson interval; spec declares uncertainty.confidence={}",
            HONORED_CONFIDENCE,
            spec.uncertainty.confidence
        );
    }
    let required = required_grader_kind(runner_kind);
    if !spec
        .graders
        .graders
        .iter()
        .any(|grader| grader.kind == required)
    {
        let article = if matches!(required, GraderKind::Agentic) {
            "an"
        } else {
            "a"
        };
        anyhow::bail!(
            "{label} runner requires {article} {required:?} grader declared in graders.graders"
        );
    }
    Ok(())
}

fn runner_kind_label(kind: RunnerKind) -> &'static str {
    match kind {
        RunnerKind::KeyRecall => "key_recall",
        RunnerKind::PromptBenchmark => "prompt_benchmark",
        RunnerKind::AgenticJudge => "agentic_judge",
        RunnerKind::HarborTask => "harbor_task",
    }
}

fn run_runner(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
    options: &RunOptions,
) -> anyhow::Result<EvalReport> {
    if options.prompt_model.is_some() && runner.kind != RunnerKind::PromptBenchmark {
        anyhow::bail!("model override can only be used with a prompt_benchmark runner");
    }
    match runner.kind {
        RunnerKind::KeyRecall => run_key_recall(spec, runner, spec_path, out),
        RunnerKind::PromptBenchmark => run_prompt_benchmark(spec, runner, spec_path, out, options),
        RunnerKind::AgenticJudge => run_agentic_judge(spec, runner, spec_path, out),
        RunnerKind::HarborTask => run_harbor_task(spec, runner, spec_path, out),
    }
}

fn run_key_recall(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
) -> anyhow::Result<EvalReport> {
    preflight_spec(spec, RunnerKind::KeyRecall)?;

    match &runner.corpus {
        CorpusSpec::DaedalusTrials {
            arena_dir,
            trials_jsonl,
            candidate_id,
            tasks,
        } => run_key_recall_daedalus(
            spec,
            runner,
            spec_path,
            out,
            arena_dir,
            trials_jsonl,
            candidate_id,
            tasks,
        ),
        CorpusSpec::CerberusReceiptBundles {
            candidate_id,
            tasks,
        } => run_key_recall_cerberus_receipts(spec, runner, spec_path, out, candidate_id, tasks),
        CorpusSpec::PromptBenchmark { .. }
        | CorpusSpec::AgenticJudge { .. }
        | CorpusSpec::HarborTasks { .. } => {
            anyhow::bail!("key_recall runner requires a key-recall corpus source")
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_key_recall_daedalus(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
    arena_dir: &str,
    trials_jsonl: &str,
    candidate_id: &str,
    tasks: &[String],
) -> anyhow::Result<EvalReport> {
    let arena_dir_resolution = resolve_spec_path_with_alias(spec_path, arena_dir);
    let trials_jsonl_resolution = resolve_spec_path_with_alias(spec_path, trials_jsonl);
    let arena_dir = arena_dir_resolution.path;
    let trials_jsonl = trials_jsonl_resolution.path;
    let selected_tasks: HashSet<&str> = tasks.iter().map(String::as_str).collect();
    let mut seen_tasks: HashSet<String> = HashSet::new();

    let file = std::fs::File::open(&trials_jsonl)
        .with_context(|| format!("opening trials corpus {}", trials_jsonl.display()))?;
    let reader = std::io::BufReader::new(file);

    let mut task_results = Vec::new();
    let mut total_matched = 0u64;
    let mut total_expected = 0u64;
    let mut total_disputed = 0u64;
    let total_recoverable = 0u64;
    let mut selected_trial_count = 0u64;

    for (line_no, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "reading line {} from {}",
                line_no + 1,
                trials_jsonl.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let trial: DaedalusTrial = serde_json::from_str(&line).with_context(|| {
            format!(
                "parsing line {} from {} as a Daedalus trial",
                line_no + 1,
                trials_jsonl.display()
            )
        })?;
        if trial.candidate_id != *candidate_id {
            continue;
        }
        if !selected_tasks.is_empty() && !selected_tasks.contains(trial.task_id.as_str()) {
            continue;
        }
        selected_trial_count += 1;
        seen_tasks.insert(trial.task_id.clone());

        let key_path = arena_dir
            .join("tasks")
            .join(&trial.task_id)
            .join("tests")
            .join("expected.json");
        let expected = ExpectedKey::from_path(&key_path)
            .with_context(|| format!("loading scorer key {}", key_path.display()))?;
        let candidate_rows = trial.findings.clone().unwrap_or_default();
        let task_score = grade_key_recall_task(&candidate_rows, &expected);

        total_matched += task_score.matched;
        total_expected += task_score.expected_defects;
        total_disputed += task_score.false_positives;

        task_results.push(TaskResult {
            task_id: trial.task_id,
            run_id: trial.run_id,
            trial: trial.trial,
            candidate_id: trial.candidate_id,
            candidate_kind: trial.candidate_kind,
            arena_id: trial.arena_id,
            arena_version: trial.arena_version,
            key: key_path.display().to_string(),
            findings: candidate_rows.len(),
            dropped_invalid: 0,
            matched: task_score.matched,
            matched_ids: task_score.grade.matched_ids,
            missed: task_score.missed,
            missed_ids: task_score.grade.missed_ids,
            disputed: task_score.false_positives,
            false_positives: task_score.false_positives,
            recoverable_misses: 0,
            expected_defects: task_score.expected_defects,
            daedalus_reward: trial.reward,
            daedalus_recall: trial.recall,
            daedalus_false_positives: trial.false_positives,
            error: trial.error,
            scorer_error: trial.scorer_error,
            artifacts: trial.artifacts,
            artifact: None,
            receipt_bundle: None,
            receipt_harness: None,
            receipt_model: None,
            receipt_validation: None,
            receipt_trusted_for_posting: None,
        });
    }

    if selected_trial_count == 0 {
        anyhow::bail!(
            "no trials found for candidate {:?} in {}",
            candidate_id,
            trials_jsonl.display()
        );
    }
    if !selected_tasks.is_empty() {
        let missing: Vec<_> = tasks
            .iter()
            .filter(|task| !seen_tasks.contains(*task))
            .cloned()
            .collect();
        if !missing.is_empty() {
            anyhow::bail!(
                "candidate {:?} has no selected trials for tasks: {}",
                candidate_id,
                missing.join(", ")
            );
        }
    }

    let score = wilson_score("pr_review_key_recall", total_matched, total_expected);
    let pass_k = compute_pass_k(&task_results);
    let evidence_path = out.join("task-results.json");
    write_json(
        &evidence_path,
        &SpecRunEvidence {
            schema_version: "crucible.spec_run_evidence.v1",
            spec_id: spec_id(spec, spec_path),
            spec: spec_path.display().to_string(),
            runner: runner.kind,
            corpus: CorpusEvidence::DaedalusTrials {
                arena_dir: arena_dir.display().to_string(),
                trials_jsonl: trials_jsonl.display().to_string(),
                declared_arena_dir: arena_dir_resolution.declared,
                declared_trials_jsonl: trials_jsonl_resolution.declared,
                arena_dir_alias: arena_dir_resolution.alias.map(str::to_string),
                trials_jsonl_alias: trials_jsonl_resolution.alias.map(str::to_string),
                candidate_id: candidate_id.to_string(),
                selected_tasks: tasks.to_vec(),
            },
            score: &score,
            totals: Totals {
                trials: selected_trial_count,
                matched: total_matched,
                expected_defects: total_expected,
                disputed: total_disputed,
                recoverable_misses: total_recoverable,
            },
            tasks: &task_results,
            pass_k: pass_k.as_ref(),
        },
    )?;

    let mut notes = vec![
        "Executed from a declared Crucible EvalSpec runner, not a built-in receipt.".to_string(),
        format!(
            "Selected {} Daedalus trial(s) for candidate {:?} and graded them against Harbor scorer keys.",
            selected_trial_count, candidate_id
        ),
    ];
    match &pass_k {
        Some(pk) => notes.push(format!(
            "pass^{}: {}/{} tasks fully matched the key on every trial ({:.1}%, {:.0}% CI [{:.1}%, {:.1}%]; Wilson over tasks, not trials).",
            pk.k,
            pk.score.successes,
            pk.score.n,
            pk.score.point.unwrap_or(0.0) * 100.0,
            pk.score.confidence * 100.0,
            pk.score.lower * 100.0,
            pk.score.upper * 100.0,
        )),
        None => notes.push(
            "pass^k not reported: tasks in this selection do not share a uniform trial count ≥ 2."
                .to_string(),
        ),
    }

    Ok(EvalReport {
        id: spec_id(spec, spec_path),
        title: if spec.task.is_empty() {
            "Declared eval spec".to_string()
        } else {
            spec.task.clone()
        },
        score,
        artifacts: vec![
            spec_path.display().to_string(),
            evidence_path.display().to_string(),
        ],
        notes,
    })
}

fn run_key_recall_cerberus_receipts(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
    candidate_id: &str,
    tasks: &[CerberusReceiptTask],
) -> anyhow::Result<EvalReport> {
    if tasks.is_empty() {
        anyhow::bail!("cerberus_receipt_bundles corpus must declare at least one task");
    }

    let mut task_results = Vec::new();
    let mut receipt_evidence = Vec::new();
    let mut total_matched = 0u64;
    let mut total_expected = 0u64;
    let mut total_disputed = 0u64;
    let total_recoverable = 0u64;

    for task in tasks {
        let artifact_path = resolve_spec_path(spec_path, &task.artifact);
        let receipt_path = resolve_spec_path(spec_path, &task.receipt_bundle);
        let expected_path = resolve_spec_path(spec_path, &task.expected);

        let receipt = load_cerberus_receipt_bundle(&receipt_path)?;
        validate_cerberus_receipt(&receipt, &receipt_path)?;
        let artifact_uri_matches =
            receipt_artifact_uri_matches(&receipt.artifact_uri, &task.artifact, &artifact_path);

        let findings = findings_from_artifact(&artifact_path)
            .with_context(|| format!("loading artifact {}", artifact_path.display()))?;
        let total_findings = findings.len();
        let valid: Vec<_> = findings.into_iter().filter(schema_valid).collect();
        let dropped_invalid = total_findings - valid.len();
        let candidate_rows = to_key_findings(&valid);
        let expected = ExpectedKey::from_path(&expected_path)
            .with_context(|| format!("loading scorer key {}", expected_path.display()))?;
        let task_score = grade_key_recall_task(&candidate_rows, &expected);

        total_matched += task_score.matched;
        total_expected += task_score.expected_defects;
        total_disputed += task_score.false_positives;

        receipt_evidence.push(CerberusReceiptEvidence {
            task_id: task.task_id.clone(),
            artifact: artifact_path.display().to_string(),
            receipt_bundle: receipt_path.display().to_string(),
            receipt_artifact_uri: receipt.artifact_uri.clone(),
            artifact_uri_matches,
            harness: receipt.harness.clone(),
            model: receipt.model.clone(),
            validation_status: receipt.validation.status.clone(),
            trusted_for_posting: receipt.validation.trusted_for_posting,
        });

        task_results.push(TaskResult {
            task_id: task.task_id.clone(),
            run_id: format!("cerberus:{}", receipt.artifact_id),
            trial: None,
            candidate_id: candidate_id.to_string(),
            candidate_kind: Some("cerberus".to_string()),
            arena_id: None,
            arena_version: None,
            key: expected_path.display().to_string(),
            findings: candidate_rows.len(),
            dropped_invalid,
            matched: task_score.matched,
            matched_ids: task_score.grade.matched_ids,
            missed: task_score.missed,
            missed_ids: task_score.grade.missed_ids,
            disputed: task_score.false_positives,
            false_positives: task_score.false_positives,
            recoverable_misses: 0,
            expected_defects: task_score.expected_defects,
            daedalus_reward: None,
            daedalus_recall: None,
            daedalus_false_positives: None,
            error: None,
            scorer_error: None,
            artifacts: None,
            artifact: Some(artifact_path.display().to_string()),
            receipt_bundle: Some(receipt_path.display().to_string()),
            receipt_harness: Some(receipt.harness),
            receipt_model: receipt.model,
            receipt_validation: Some(receipt.validation.status),
            receipt_trusted_for_posting: Some(receipt.validation.trusted_for_posting),
        });
    }

    let score = wilson_score("pr_review_key_recall", total_matched, total_expected);
    let evidence_path = out.join("task-results.json");
    write_json(
        &evidence_path,
        &SpecRunEvidence {
            schema_version: "crucible.spec_run_evidence.v1",
            spec_id: spec_id(spec, spec_path),
            spec: spec_path.display().to_string(),
            runner: runner.kind,
            corpus: CorpusEvidence::CerberusReceiptBundles {
                candidate_id: candidate_id.to_string(),
                tasks: receipt_evidence,
            },
            score: &score,
            totals: Totals {
                trials: tasks.len() as u64,
                matched: total_matched,
                expected_defects: total_expected,
                disputed: total_disputed,
                recoverable_misses: total_recoverable,
            },
            tasks: &task_results,
            // Cerberus receipt bundles are one artifact per task, not repeated
            // trials — there is no k to measure consistency over here.
            pass_k: None,
        },
    )?;

    Ok(EvalReport {
        id: spec_id(spec, spec_path),
        title: if spec.task.is_empty() {
            "Declared eval spec".to_string()
        } else {
            spec.task.clone()
        },
        score,
        artifacts: vec![spec_path.display().to_string(), evidence_path.display().to_string()],
        notes: vec![
            "Executed from a declared Crucible EvalSpec runner, not a built-in receipt.".to_string(),
            format!(
                "Selected {} fresh Cerberus receipt bundle task(s) for candidate {:?} and graded them against Harbor scorer keys.",
                tasks.len(), candidate_id
            ),
        ],
    })
}

/// Default per-task `harbor run` wall-clock budget: Harbor's own task.toml
/// declares independent ~600s timeouts for environment build, agent setup,
/// agent execution, and verification, so a worst-case task can spend several
/// times that before Harbor itself gives up. 30 minutes gives real tasks room
/// without letting a wedged container hang the whole benchmark run.
const HARBOR_DEFAULT_JOB_TIMEOUT_MS: u64 = 1_800_000;

fn run_harbor_task(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
) -> anyhow::Result<EvalReport> {
    preflight_spec(spec, RunnerKind::HarborTask)?;

    let CorpusSpec::HarborTasks { config, tasks } = &runner.corpus else {
        anyhow::bail!("harbor_task runner requires corpus.source=harbor_tasks");
    };
    if tasks.is_empty() {
        anyhow::bail!("harbor_tasks corpus must declare at least one task");
    }
    check_harbor_available()?;

    let jobs_root = out.join("harbor-jobs");
    let mut task_results = Vec::with_capacity(tasks.len());
    for task in tasks {
        task_results.push(run_one_harbor_task(spec_path, config, task, &jobs_root)?);
    }

    let passed = task_results.iter().filter(|result| result.passed).count() as u64;
    let score = wilson_score("harbor_reward_pass_rate", passed, tasks.len() as u64);
    let evidence_path = out.join("harbor-run.json");
    write_json(
        &evidence_path,
        &HarborRunEvidence {
            schema_version: "crucible.harbor_run_evidence.v1",
            spec_id: spec_id(spec, spec_path),
            spec: spec_path.display().to_string(),
            runner: runner.kind,
            agent: config.agent.clone(),
            model: config.model.clone(),
            resource_envelope: config.resource_envelope,
            score: &score,
            totals: PromptTotals {
                tasks: tasks.len() as u64,
                passed,
                failed: tasks.len() as u64 - passed,
            },
            tasks: &task_results,
        },
    )?;

    Ok(EvalReport {
        id: spec_id(spec, spec_path),
        title: if spec.task.is_empty() {
            "Harbor task benchmark".to_string()
        } else {
            spec.task.clone()
        },
        score,
        artifacts: vec![spec_path.display().to_string(), evidence_path.display().to_string()],
        notes: vec![
            "Executed from a Crucible-authored harbor_task runner, shelling out to the `harbor` CLI per task (backlog/Powder crucible-034).".to_string(),
            format!(
                "Ran {} Harbor task(s) under agent {:?} and graded on Harbor's own verifier reward (>= 1.0 counted as pass).",
                tasks.len(), config.agent
            ),
        ],
    })
}

/// Refuse before spawning any subprocess when `harbor` or Docker aren't on
/// this machine, with an actionable message instead of a raw spawn failure or
/// a confusing mid-run Docker error.
fn check_harbor_available() -> anyhow::Result<()> {
    let harbor_ok = Command::new("harbor")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !harbor_ok {
        anyhow::bail!(
            "harbor_task runner requires the `harbor` CLI on PATH (e.g. `uv tool install harbor` or `pip install harbor`); `harbor --version` did not succeed"
        );
    }
    let docker_ok = Command::new("docker")
        .arg("info")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !docker_ok {
        anyhow::bail!(
            "harbor_task runner requires a running Docker daemon (e.g. Colima); `docker info` did not succeed"
        );
    }
    Ok(())
}

/// Refuse a task directory outside `$HOME`: Colima's default configuration
/// only bind-mounts `$HOME` into its Docker VM, so a task directory outside it
/// resolves as empty inside the container and Harbor fails with
/// `RewardFileNotFoundError` — a confusing failure far from its real cause.
/// Caught here, before any subprocess spawns.
fn require_under_home(resolved: &Path) -> anyhow::Result<()> {
    let home = std::env::var("HOME").context(
        "harbor_task runner requires $HOME to be set (Colima mounts $HOME, not arbitrary paths)",
    )?;
    let home = canonicalize_existing(Path::new(&home));
    if !resolved.starts_with(&home) {
        anyhow::bail!(
            "harbor task_dir {} resolves outside $HOME ({}); Colima only bind-mounts $HOME into its Docker VM, so a task directory elsewhere fails inside the container with RewardFileNotFoundError — move the task (and this checkout) under $HOME",
            resolved.display(),
            home.display()
        );
    }
    Ok(())
}

/// Job name every harbor trial runs under. A single fixed name, not a
/// per-invocation unique id: [`prepare_harbor_job_dir`] clears this slot
/// before every run, so reuse is intentional (the isolation guarantee is "no
/// artifact survives across a reused slot", not "every slot is unique").
const HARBOR_JOB_NAME: &str = "run";

/// The job output directory a harbor task's trial writes to:
/// `jobs_root/<task_id>/<HARBOR_JOB_NAME>`. Pure path construction, no I/O —
/// [`prepare_harbor_job_dir`] is the version that also enforces the
/// isolation contract (docs/AGENTS.md "Trial isolation"): distinct task ids
/// get distinct, non-overlapping directories under `jobs_root`.
fn harbor_job_dir(jobs_root: &Path, task_id: &str) -> std::path::PathBuf {
    jobs_root.join(task_id).join(HARBOR_JOB_NAME)
}

/// Prepare a harbor trial's job output directory: create `jobs_root/task_id`
/// and clear any stale `HARBOR_JOB_NAME` slot left by a prior run of the same
/// task id, so a new trial never inherits artifacts a previous trial wrote
/// there. This is the isolation contract's "no access to
/// sibling-trial or prior-run artifacts" and "fresh working directory"
/// guarantees, mechanically enforced (docs/AGENTS.md "Trial isolation") —
/// exercised directly by this module's `harbor_job_directory_clears_prior_trial_artifacts_before_reuse`
/// and `harbor_job_directories_are_disjoint_across_task_ids` tests, without
/// needing a live `harbor`/Docker install.
fn prepare_harbor_job_dir(jobs_root: &Path, task_id: &str) -> anyhow::Result<std::path::PathBuf> {
    let jobs_dir = jobs_root.join(task_id);
    std::fs::create_dir_all(&jobs_dir)
        .with_context(|| format!("creating harbor jobs directory {}", jobs_dir.display()))?;
    let job_dir = harbor_job_dir(jobs_root, task_id);
    if job_dir.exists() {
        std::fs::remove_dir_all(&job_dir).with_context(|| {
            format!("clearing stale harbor job directory {}", job_dir.display())
        })?;
    }
    Ok(job_dir)
}

fn run_one_harbor_task(
    spec_path: &Path,
    config: &HarborRunConfig,
    task: &HarborTaskSpec,
    jobs_root: &Path,
) -> anyhow::Result<HarborTaskResult> {
    let task_dir = resolve_spec_path(spec_path, &task.task_dir);
    if !task_dir.exists() {
        anyhow::bail!(
            "harbor task {:?} declares task_dir {} which does not exist",
            task.task_id,
            task_dir.display()
        );
    }
    require_under_home(&task_dir)?;

    let jobs_dir = jobs_root.join(&task.task_id);
    let job_dir = prepare_harbor_job_dir(jobs_root, &task.task_id)?;

    let mut command = Command::new("harbor");
    command
        .arg("run")
        .arg("-p")
        .arg(&task_dir)
        .arg("-a")
        .arg(&config.agent)
        .arg("-o")
        .arg(&jobs_dir)
        .arg("--job-name")
        .arg(HARBOR_JOB_NAME)
        // Auto-confirm host-environment-access prompts: this subprocess has
        // no interactive stdin, so an unconfirmed prompt would otherwise hang
        // until the timeout below kills it rather than failing fast.
        .arg("-y");
    if let Some(model) = &config.model {
        command.arg("-m").arg(model);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let started = Instant::now();
    let mut child = command
        .spawn()
        .with_context(|| format!("spawning `harbor run` for task {:?}", task.task_id))?;
    let timeout_ms = config
        .job_timeout_ms
        .unwrap_or(HARBOR_DEFAULT_JOB_TIMEOUT_MS);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let timed_out = loop {
        if let Some(_status) = child
            .try_wait()
            .context("checking harbor run subprocess status")?
        {
            break false;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break true;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let output = child
        .wait_with_output()
        .context("collecting harbor run subprocess output")?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

    let log_path = jobs_dir.join("harbor-run.log");
    std::fs::write(
        &log_path,
        format!(
            "exit_status={:?}\ntimed_out={timed_out}\n--- stdout ---\n{}\n--- stderr ---\n{}\n",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ),
    )
    .with_context(|| format!("writing harbor run log {}", log_path.display()))?;

    if timed_out {
        anyhow::bail!(
            "harbor run for task {:?} did not finish within {timeout_ms}ms and was killed; see {}",
            task.task_id,
            log_path.display()
        );
    }

    let trial_result = read_harbor_trial_result(&job_dir).with_context(|| {
        format!(
            "reading harbor trial result for task {:?} under {}",
            task.task_id,
            job_dir.display()
        )
    })?;
    let trial_dir = trial_result.trial_dir;
    let result_json = trial_result.result_json;

    let outcome = derive_harbor_outcome(&result_json, &task.task_id)?;
    let exception = outcome.exception;
    let reward = outcome.reward;
    let reward_breakdown = outcome.reward_breakdown;
    let passed = outcome.passed;

    let harbor_task_ref = result_json
        .get("task_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&task.task_dir)
        .to_string();

    let verifier_summary =
        std::fs::read_to_string(trial_dir.join("verifier").join("test-stdout.txt"))
            .ok()
            .map(|text| truncate_for_summary(&text, 2000));

    let mut artifacts = Vec::new();
    for candidate in ["verifier/reward.txt", "verifier/test-stdout.txt"] {
        let path = trial_dir.join(candidate);
        if path.exists() {
            artifacts.push(path.display().to_string());
        }
    }
    let artifacts_manifest = trial_dir.join("artifacts").join("manifest.json");
    if artifacts_manifest.exists() {
        artifacts.push(artifacts_manifest.display().to_string());
    }

    Ok(HarborTaskResult {
        task_id: task.task_id.clone(),
        task_dir: task_dir.display().to_string(),
        agent: config.agent.clone(),
        harbor_task_ref,
        passed,
        reward,
        reward_breakdown,
        latency_ms,
        verifier_summary,
        artifacts,
        exception,
        evidence_json: result_json,
    })
}

#[derive(Debug)]
struct HarborOutcome {
    passed: bool,
    reward: f64,
    reward_breakdown: serde_json::Value,
    exception: Option<serde_json::Value>,
}

/// Derive pass/fail and the primary reward from one Harbor trial `result.json`
/// (pure function over already-parsed JSON, so this is testable without
/// spawning `harbor` or Docker). `exception_info` non-null always fails the
/// task regardless of any reward value Harbor still reported. Otherwise the
/// primary reward is `verifier_result.rewards["reward"]` — Harbor's own
/// convention, also the shape this runner's fixtures declare — falling back
/// to the sole entry when a task names its reward differently but declares
/// only one; a task with several differently-named rewards and no `"reward"`
/// key is refused rather than silently guessing which one is primary.
/// `passed` requires the primary reward to be `>= 1.0` (full credit) — partial
/// reward is recorded on the row but does not count toward the Wilson
/// proportion score `preflight_spec` requires for this runner.
fn derive_harbor_outcome(
    result_json: &serde_json::Value,
    task_id: &str,
) -> anyhow::Result<HarborOutcome> {
    let exception = result_json
        .get("exception_info")
        .filter(|v| !v.is_null())
        .cloned();
    let rewards = result_json
        .get("verifier_result")
        .and_then(|v| v.get("rewards"))
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    let reward_breakdown = serde_json::Value::Object(rewards.clone());
    if exception.is_some() {
        return Ok(HarborOutcome {
            passed: false,
            reward: 0.0,
            reward_breakdown,
            exception,
        });
    }
    let reward = match rewards.get("reward").and_then(serde_json::Value::as_f64) {
        Some(reward) => reward,
        None if rewards.len() == 1 => rewards
            .values()
            .next()
            .and_then(serde_json::Value::as_f64)
            .with_context(|| {
                format!(
                    "task {task_id:?}: harbor verifier_result.rewards' sole entry is not numeric: {rewards:?}"
                )
            })?,
        None => anyhow::bail!(
            "task {task_id:?}: harbor verifier_result.rewards names no \"reward\" key and has {} entries, so the primary reward is ambiguous: {rewards:?}",
            rewards.len()
        ),
    };
    Ok(HarborOutcome {
        passed: reward >= 1.0,
        reward,
        reward_breakdown,
        exception: None,
    })
}

fn truncate_for_summary(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    format!("{truncated}\n...[truncated]")
}

#[derive(Debug)]
struct HarborTrialResult {
    trial_dir: PathBuf,
    result_json: serde_json::Value,
}

/// Locate and read the single trial's `result.json` under a Harbor job
/// directory. One `harbor run -p <task_dir>` invocation with the default
/// single attempt produces exactly one trial subdirectory; anything else (no
/// subdirectory, or more than one) is reported as a distinct, named error
/// rather than silently picking one.
fn read_harbor_trial_result(job_dir: &Path) -> anyhow::Result<HarborTrialResult> {
    let mut trial_dirs = Vec::new();
    for entry in std::fs::read_dir(job_dir)
        .with_context(|| format!("reading harbor job directory {}", job_dir.display()))?
    {
        let entry = entry.context("reading harbor job directory entry")?;
        if entry
            .file_type()
            .context("reading entry file type")?
            .is_dir()
        {
            trial_dirs.push(entry.path());
        }
    }
    match trial_dirs.len() {
        0 => anyhow::bail!("no trial subdirectory found under {}", job_dir.display()),
        1 => {}
        n => anyhow::bail!(
            "expected exactly one trial subdirectory under {} (single task, single attempt), found {n}",
            job_dir.display()
        ),
    }
    let trial_dir = trial_dirs.remove(0);
    let result_path = trial_dir.join("result.json");
    let bytes = std::fs::read(&result_path)
        .with_context(|| format!("reading harbor trial result {}", result_path.display()))?;
    let result_json: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing harbor trial result {}", result_path.display()))?;
    Ok(HarborTrialResult {
        trial_dir,
        result_json,
    })
}

#[derive(Debug, Serialize)]
struct HarborRunEvidence<'a> {
    schema_version: &'static str,
    spec_id: String,
    spec: String,
    runner: RunnerKind,
    agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    /// The sandbox's declared [`crucible_core::ResourceEnvelope`] (backlog
    /// 974), when the corpus author configured one.
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_envelope: Option<ResourceEnvelope>,
    score: &'a Score,
    totals: PromptTotals,
    tasks: &'a [HarborTaskResult],
}

#[derive(Debug, Serialize)]
struct HarborTaskResult {
    task_id: String,
    task_dir: String,
    agent: String,
    harbor_task_ref: String,
    passed: bool,
    reward: f64,
    reward_breakdown: serde_json::Value,
    latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    verifier_summary: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    artifacts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exception: Option<serde_json::Value>,
    evidence_json: serde_json::Value,
}

fn run_prompt_benchmark(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
    options: &RunOptions,
) -> anyhow::Result<EvalReport> {
    preflight_spec(spec, RunnerKind::PromptBenchmark)?;

    let CorpusSpec::PromptBenchmark { config, tasks } = &runner.corpus else {
        anyhow::bail!("prompt_benchmark runner requires corpus.source=prompt_benchmark");
    };
    // A malformed Regex expectation fails here, before any model call is
    // made — not mid-grading after tasks have already spent real API calls.
    check_prompt_regexes(tasks)?;
    check_prompt_tracked_ids(tasks)?;
    let effective_config = prompt_config_with_overrides(config, options);
    let client = OpenRouterClient::from_config(&effective_config)?;
    run_prompt_benchmark_with_client(
        spec,
        runner,
        spec_path,
        out,
        &effective_config,
        tasks,
        &client,
    )
}

fn prompt_config_with_overrides(
    config: &PromptModelConfig,
    options: &RunOptions,
) -> PromptModelConfig {
    let mut config = config.clone();
    if let Some(model) = options
        .prompt_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        config.model = model.to_string();
    }
    if let Some(system_prompt) = options
        .prompt_system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|system_prompt| !system_prompt.is_empty())
    {
        config.system_prompt = system_prompt.to_string();
    }
    if let Some(max_output_units) = options.prompt_max_output_units {
        config.max_output_units = Some(max_output_units);
    }
    if let Some(temperature) = options.prompt_temperature {
        config.temperature = Some(temperature);
    }
    config
}

/// Precompile every declared `Regex` expectation's pattern, failing on the
/// first that does not compile. Shared by [`run_prompt_benchmark`] (a hard
/// refusal before any model call) and `crucible validate`
/// (`crate::validate`, which reports the same failure as a named error
/// instead of a load-time bail).
pub(crate) fn check_prompt_regexes(tasks: &[PromptBenchmarkTask]) -> anyhow::Result<()> {
    for task in tasks {
        if let PromptExpectation::Regex { pattern } = &task.expectation {
            compile_expectation_regex(pattern)
                .with_context(|| format!("task {:?}", task.task_id))?;
        }
        for check in &task.tracked {
            if let PromptExpectation::Regex { pattern } = &check.expectation {
                compile_expectation_regex(pattern).with_context(|| {
                    format!("task {:?} tracked check {:?}", task.task_id, check.id)
                })?;
            }
        }
    }
    Ok(())
}

/// Refuse duplicate tracked-check ids within one prompt task. The id scope is
/// per task, so the same tracked id may appear on different tasks.
pub(crate) fn check_prompt_tracked_ids(tasks: &[PromptBenchmarkTask]) -> anyhow::Result<()> {
    for task in tasks {
        let mut seen = HashSet::new();
        for check in &task.tracked {
            if !seen.insert(check.id.as_str()) {
                anyhow::bail!(
                    "task {:?} declares duplicate tracked id {:?}",
                    task.task_id,
                    check.id
                );
            }
        }
    }
    Ok(())
}

/// Bounded worker width for concurrent OpenRouter calls across a prompt
/// benchmark's tasks. These calls are network-bound (waiting on the model
/// provider), not CPU-bound, so a small fixed width is the right knob — high
/// enough to erase most of the linear wall-clock cost, low enough not to
/// hammer OpenRouter's per-key rate limits.
const PROMPT_TASK_CONCURRENCY: usize = 4;

fn run_prompt_benchmark_with_client(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
    config: &PromptModelConfig,
    tasks: &[PromptBenchmarkTask],
    model_client: &(dyn ModelClient + Sync),
) -> anyhow::Result<EvalReport> {
    if config.provider != ModelProvider::OpenRouter {
        anyhow::bail!(
            "unsupported prompt benchmark provider: {:?}",
            config.provider
        );
    }
    if tasks.is_empty() {
        anyhow::bail!("prompt_benchmark corpus must declare at least one task");
    }

    let task_results = run_prompt_tasks_concurrently(spec_path, config, tasks, model_client)?;
    let passed = task_results.iter().filter(|result| result.passed).count() as u64;

    let score = wilson_score("prompt_rubric_pass_rate", passed, tasks.len() as u64);
    let evidence_path = out.join("prompt-run.json");
    write_json(
        &evidence_path,
        &PromptRunEvidence {
            schema_version: "crucible.prompt_run_evidence.v1",
            spec_id: spec_id(spec, spec_path),
            spec: spec_path.display().to_string(),
            runner: runner.kind,
            provider: config.provider,
            model: config.model.clone(),
            temperature: config.temperature,
            max_output_units: config.max_output_units,
            harness: config.harness.clone(),
            tool_allowlist: config.tool_allowlist.clone(),
            system_prompt_hash: stable_hash(&[&config.system_prompt]),
            score: &score,
            totals: PromptTotals {
                tasks: tasks.len() as u64,
                passed,
                failed: tasks.len() as u64 - passed,
            },
            tasks: &task_results,
        },
    )?;

    Ok(EvalReport {
        id: spec_id(spec, spec_path),
        title: if spec.task.is_empty() {
            "Prompt benchmark".to_string()
        } else {
            spec.task.clone()
        },
        score,
        artifacts: vec![spec_path.display().to_string(), evidence_path.display().to_string()],
        notes: vec![
            "Executed from a Crucible-authored prompt benchmark runner with a live model boundary, not Threshold."
                .to_string(),
            format!(
                "Ran {} prompt task(s) against {:?}/{} and graded deterministic text rubrics.",
                tasks.len(), config.provider, config.model
            ),
        ],
    })
}

/// Execute every prompt task's model call with up to
/// [`PROMPT_TASK_CONCURRENCY`] calls in flight at once, returning results in
/// the caller's task order regardless of which worker finished first.
///
/// Workers pull the next unclaimed task index off a shared atomic counter
/// (simple work-stealing) rather than a static chunk-per-thread split, so one
/// slow task doesn't stall a worker that could otherwise pick up more work.
/// The first task error observed while collecting results still aborts the
/// whole run with no evidence written — the same contract the old sequential
/// loop had — though a handful of already-in-flight sibling calls may
/// complete (and spend) before that error surfaces; that trade is inherent to
/// running calls concurrently and is bounded by the concurrency width.
fn run_prompt_tasks_concurrently(
    spec_path: &Path,
    config: &PromptModelConfig,
    tasks: &[PromptBenchmarkTask],
    model_client: &(dyn ModelClient + Sync),
) -> anyhow::Result<Vec<PromptTaskResult>> {
    let concurrency = PROMPT_TASK_CONCURRENCY.min(tasks.len()).max(1);
    let next_index = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<anyhow::Result<PromptTaskResult>>>> =
        (0..tasks.len()).map(|_| Mutex::new(None)).collect();

    thread::scope(|scope| {
        for _ in 0..concurrency {
            scope.spawn(|| loop {
                let idx = next_index.fetch_add(1, Ordering::SeqCst);
                if idx >= tasks.len() {
                    return;
                }
                let outcome = run_one_prompt_task(spec_path, config, &tasks[idx], model_client);
                *slots[idx]
                    .lock()
                    .expect("prompt task result slot mutex poisoned") = Some(outcome);
            });
        }
    });

    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .expect("prompt task result slot mutex poisoned")
                .expect("every prompt task index is claimed by exactly one worker")
        })
        .collect()
}

fn run_one_prompt_task(
    spec_path: &Path,
    config: &PromptModelConfig,
    task: &PromptBenchmarkTask,
    model_client: &dyn ModelClient,
) -> anyhow::Result<PromptTaskResult> {
    let user_prompt = prompt_text_for_task(spec_path, task)?;
    let started = Instant::now();
    let response = model_client.complete(ModelRequest {
        model: &config.model,
        system_prompt: &config.system_prompt,
        user_prompt: &user_prompt,
        max_output_units: config.max_output_units,
        temperature: config.temperature,
    })?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let task_passed = prompt_expectation_passes(&response.output, &task.expectation)
        .with_context(|| format!("grading prompt task {:?}", task.task_id))?;
    let tracked_results = task
        .tracked
        .iter()
        .map(|check| {
            let passed = prompt_expectation_passes(&response.output, &check.expectation)
                .with_context(|| {
                    format!(
                        "grading tracked prompt check {:?} on task {:?}",
                        check.id, task.task_id
                    )
                })?;
            Ok(TrackedCheckResult {
                id: check.id.clone(),
                passed,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let expectation_value = expectation_value(&task.expectation);
    Ok(PromptTaskResult {
        task_id: task.task_id.clone(),
        class: task.class.clone(),
        context_file: task.context_file.clone(),
        prompt_hash: stable_hash(&[&config.system_prompt, &user_prompt]),
        rubric_hash: stable_hash(&[expectation_kind(&task.expectation), &expectation_value]),
        expectation: task.expectation.clone(),
        passed: task_passed,
        tracked_results,
        output: response.output,
        latency_ms,
        response_id: response.response_id,
        requested_model: config.model.clone(),
        response_model: response.response_model,
        input_units: response.input_units,
        output_units: response.output_units,
        total_units: response.total_units,
        cost_usd: response.cost_usd,
    })
}

fn prompt_text_for_task(spec_path: &Path, task: &PromptBenchmarkTask) -> anyhow::Result<String> {
    let Some(context_file) = task.context_file.as_deref() else {
        return Ok(task.prompt.clone());
    };
    let context_path = resolve_spec_path(spec_path, context_file);
    let context = std::fs::read_to_string(&context_path)
        .with_context(|| format!("reading prompt context file {}", context_path.display()))?;
    Ok(format!(
        "Context document:\n{context}\n\nTask:\n{}",
        task.prompt
    ))
}

/// Judge protocol suffix appended to every judge call's system prompt so the
/// response is parseable regardless of what the operator wrote in
/// `judge_prompt`: reasoning first, then exactly one `VERDICT: PASS`,
/// `VERDICT: FAIL`, or `VERDICT: UNKNOWN` line as the FINAL line (report §6
/// checklist item 8: "give the judge an explicit `unknown`/
/// `insufficient_information` option"; RubricEval, arXiv:2603.25133:
/// reasoning-before-verdict adds 6.7-9.0 balanced-accuracy points over
/// verdict-first — this protocol was previously exactly backwards).
/// [`parse_judge_verdict`] is tail-anchored to match: it reads only the
/// final line, so the verdict must come last.
const JUDGE_VERDICT_PROTOCOL: &str = "\n\nRespond with your reasoning first: a short paragraph explaining how the candidate does or does not meet the rubric. Do not rubber-stamp: a candidate that fails the rubric must get VERDICT: FAIL even if it is close. Use VERDICT: UNKNOWN only when the rubric and candidate genuinely do not give you enough information to decide — never guess a PASS or FAIL you are not confident in. End your response with exactly one line, and nothing after it, in the form `VERDICT: PASS`, `VERDICT: FAIL`, or `VERDICT: UNKNOWN`.";

/// Build the judge's user prompt for one task: the rubric, an optional
/// reference exemplar, and the candidate output. The reference — when
/// present — is labeled as a known-perfect exemplar and never presented as
/// the candidate being judged; *Evaluating Scoring Bias in LLM-as-a-Judge*
/// (arXiv:2506.22316) found this reliably improves scoring accuracy across
/// judges and normalizes skewed scoring tendencies.
///
/// `cosmetic_reorder` swaps the rubric/candidate section order without
/// changing their content — used only by the format-sensitivity self-check
/// (same paper: purely cosmetic prompt perturbations move scores in
/// judge-specific directions) to probe whether that perturbation alone
/// flips the judge's verdict.
fn judge_user_prompt(task: &AgenticJudgeTask, cosmetic_reorder: bool) -> String {
    let reference_block = task
        .reference
        .as_deref()
        .map(|reference| {
            format!(
                "\n\nReference answer (a known-perfect exemplar for this rubric — NOT the candidate being judged):\n{reference}"
            )
        })
        .unwrap_or_default();
    if cosmetic_reorder {
        format!(
            "Candidate output:\n{}{reference_block}\n\nRubric:\n{}",
            task.candidate, task.rubric
        )
    } else {
        format!(
            "Rubric:\n{}{reference_block}\n\nCandidate output:\n{}",
            task.rubric, task.candidate
        )
    }
}

/// A judge's verdict on one task: pass, fail, or a genuine inability to
/// decide (report §6 item 8). `Unknown` is a distinct, first-class outcome —
/// never silently coerced to `Pass` or `Fail` for scoring or calibration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JudgeVerdict {
    Pass,
    Fail,
    Unknown,
}

impl JudgeVerdict {
    /// The binary reading of this verdict, for calibration/scoring paths
    /// that only make sense for a decisive verdict. `None` for `Unknown`.
    fn as_bool(self) -> Option<bool> {
        match self {
            JudgeVerdict::Pass => Some(true),
            JudgeVerdict::Fail => Some(false),
            JudgeVerdict::Unknown => None,
        }
    }
}

impl std::fmt::Display for JudgeVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            JudgeVerdict::Pass => "PASS",
            JudgeVerdict::Fail => "FAIL",
            JudgeVerdict::Unknown => "UNKNOWN",
        })
    }
}

/// Raw-agreement floor an agentic judge's calls against the tasks with a known
/// `expected_pass` (the deterministic tier for the same tasks) must clear to
/// unlock. Below this, the judge's score is diagnostic, not trusted — backlog
/// 012's "refuses to unlock without a CalibrationRecord that clears the
/// configured agreement threshold."
const CALIBRATION_AGREEMENT_THRESHOLD: f64 = 0.8;

fn run_agentic_judge(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
) -> anyhow::Result<EvalReport> {
    preflight_spec(spec, RunnerKind::AgenticJudge)?;

    let CorpusSpec::AgenticJudge { config, tasks } = &runner.corpus else {
        anyhow::bail!("agentic_judge runner requires corpus.source=agentic_judge");
    };
    let client = OpenRouterClient::from_credential_env(&config.credential_env)?;
    run_agentic_judge_with_client(spec, runner, spec_path, out, config, tasks, &client)
}

fn run_agentic_judge_with_client(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
    config: &AgenticJudgeConfig,
    tasks: &[AgenticJudgeTask],
    model_client: &dyn ModelClient,
) -> anyhow::Result<EvalReport> {
    if config.provider != ModelProvider::OpenRouter {
        anyhow::bail!("unsupported agentic judge provider: {:?}", config.provider);
    }
    if tasks.is_empty() {
        anyhow::bail!("agentic_judge corpus must declare at least one task");
    }

    let judge_system_prompt = format!("{}{JUDGE_VERDICT_PROTOCOL}", config.judge_prompt);
    let system_prompt_hash = stable_hash(&[&judge_system_prompt]);
    // Self-evaluation bias check (report §6's self-preference bias table):
    // does the judge share a model family with whoever generated the
    // candidates it is scoring? Checked once at construction, not per-task —
    // the answer does not depend on any one task — and surfaced on the
    // calibration record rather than gating the run.
    let self_evaluation_bias_risk = config
        .generator_model
        .as_deref()
        .is_some_and(|generator| shares_model_family(&config.model, generator));
    let mut task_results = Vec::new();
    let mut scored_successes = 0u64;
    let mut scored_n = 0u64;
    let mut unknown_scored = 0u64;
    let mut canary_notes = Vec::new();
    // Every task with a known `expected_pass` is a paired (judge, deterministic)
    // calibration item, not only the judge-gaming canary — backlog 012's
    // calibration record measures the judge against the deterministic tier on
    // the same tasks.
    let mut calibration_judge = Vec::new();
    let mut calibration_human = Vec::new();
    let mut calibration_rubric_hashes = Vec::new();
    let mut unknown_calibration = 0u64;
    // Every decisive calibration item, paired with the judge's original
    // verdict — the sample the format-sensitivity self-check re-probes with
    // a cosmetically reordered prompt (config.format_sensitivity_check).
    let mut calibration_probe_items: Vec<(&AgenticJudgeTask, bool)> = Vec::new();
    // Judge-specific run stats (report §6 item 11), distinct from any
    // candidate-generation cost this runner does not itself incur (candidates
    // here are authored strings, not live-generated).
    let mut total_cost_usd = 0.0f64;
    let mut any_cost_recorded = false;
    let mut total_latency_ms: u64 = 0;
    // Ordered record of what actually happened, task by task (backlog 030):
    // a judge_call, its parsed verdict, and — for calibration tasks — the
    // agreement/mismatch/unknown check against `expected_pass`. This is what
    // makes a failed or UNKNOWN-verdict run inspectable without re-running
    // the judge.
    let mut trace_steps: Vec<TraceStep> = Vec::new();
    let mut trace_sequence: u64 = 0;

    for task in tasks {
        let user_prompt = judge_user_prompt(task, false);
        let started = Instant::now();
        let response = model_client.complete(ModelRequest {
            model: &config.model,
            system_prompt: &judge_system_prompt,
            user_prompt: &user_prompt,
            max_output_units: None,
            temperature: config.temperature,
        })?;
        let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        total_latency_ms = total_latency_ms.saturating_add(latency_ms);
        if let Some(cost) = response.cost_usd {
            total_cost_usd += cost;
            any_cost_recorded = true;
        }
        let verdict = parse_judge_verdict(&response.output).with_context(|| {
            format!(
                "agentic judge task {:?} returned an unparseable verdict",
                task.task_id
            )
        })?;
        let rubric_hash = stable_hash(&[&task.rubric]);
        let verdict_str = match verdict {
            JudgeVerdict::Pass => "pass",
            JudgeVerdict::Fail => "fail",
            JudgeVerdict::Unknown => "unknown",
        };

        push_trace_step(
            &mut trace_steps,
            &mut trace_sequence,
            "judge_call",
            &task.task_id,
            serde_json::json!({
                "model": config.model,
                "rubric": task.rubric,
                "candidate": task.candidate,
                "latency_ms": latency_ms,
                "cost_usd": response.cost_usd,
                "response_id": response.response_id,
            }),
            None,
        );
        push_trace_step(
            &mut trace_steps,
            &mut trace_sequence,
            "verdict_parsed",
            &task.task_id,
            serde_json::json!({
                "raw_output": response.output,
                "verdict": verdict_str,
                "expected_pass": task.expected_pass,
            }),
            Some(verdict_str),
        );

        match (task.expected_pass, verdict) {
            (Some(_), JudgeVerdict::Unknown) => {
                // Diagnostic, not a mismatch: an honest "I can't tell" is not
                // the judge-gaming guard's target (rubber-stamping) and must
                // never be coerced into agreeing or disagreeing with
                // `expected_pass`. Excluded from the agreement/κ measurement.
                unknown_calibration += 1;
                calibration_rubric_hashes.push(rubric_hash.clone());
                push_trace_step(
                    &mut trace_steps,
                    &mut trace_sequence,
                    "calibration_check",
                    &task.task_id,
                    serde_json::json!({
                        "expected_pass": task.expected_pass,
                        "judge_verdict": "unknown",
                    }),
                    Some("unknown"),
                );
                canary_notes.push(format!(
                    "calibration task {:?} returned UNKNOWN — diagnostic, excluded from the agreement measurement.",
                    task.task_id
                ));
            }
            (Some(expected), verdict) => {
                let verdict_bool = verdict
                    .as_bool()
                    .expect("Unknown is handled by the arm above");
                calibration_rubric_hashes.push(rubric_hash.clone());
                let matched = expected == verdict_bool;
                push_trace_step(
                    &mut trace_steps,
                    &mut trace_sequence,
                    "calibration_check",
                    &task.task_id,
                    serde_json::json!({
                        "expected_pass": expected,
                        "judge_verdict": verdict_bool,
                        "refuse_on_mismatch": task.refuse_on_mismatch,
                    }),
                    Some(if matched { "match" } else { "mismatch" }),
                );
                if !matched {
                    if task.refuse_on_mismatch {
                        anyhow::bail!(
                            "judge-gaming guard tripped on task {:?}: expected verdict {expected} but the judge said {verdict}; refusing to trust this run",
                            task.task_id
                        );
                    }
                    calibration_judge.push(verdict_bool);
                    calibration_human.push(expected);
                    calibration_probe_items.push((task, verdict_bool));
                    canary_notes.push(format!(
                        "calibration task {:?} disagreed with the judge (expected {expected}, got {verdict}) but did not refuse the run.",
                        task.task_id
                    ));
                } else {
                    calibration_judge.push(verdict_bool);
                    calibration_human.push(expected);
                    calibration_probe_items.push((task, verdict_bool));
                    canary_notes.push(format!(
                        "calibration task {:?} matched its expected verdict.",
                        task.task_id
                    ));
                }
            }
            (None, JudgeVerdict::Unknown) => {
                // Diagnostic: never silently coerced to a pass or a fail —
                // excluded from the scored denominator entirely.
                unknown_scored += 1;
            }
            (None, verdict) => {
                scored_n += 1;
                if verdict == JudgeVerdict::Pass {
                    scored_successes += 1;
                }
            }
        }

        task_results.push(AgenticJudgeTaskResult {
            task_id: task.task_id.clone(),
            prompt_hash: stable_hash(&[&judge_system_prompt, &user_prompt]),
            rubric_hash,
            expected_pass: task.expected_pass,
            verdict: verdict_str,
            // Legacy bool mirror of `verdict`, kept for the shared
            // prompt/judge evidence ingestion path in `run_store.rs` which
            // requires every task to carry a `passed` bool. `Unknown` reads
            // as `false` here — the authoritative tri-state lives in
            // `verdict`, never re-derived from this field.
            passed: verdict == JudgeVerdict::Pass,
            output: response.output,
            latency_ms,
            response_id: response.response_id,
            requested_model: config.model.clone(),
            response_model: response.response_model,
            input_units: response.input_units,
            output_units: response.output_units,
            total_units: response.total_units,
            cost_usd: response.cost_usd,
        });
    }

    if scored_n == 0 {
        anyhow::bail!(
            "agentic_judge corpus must declare at least one task without expected_pass to score"
        );
    }

    // Format-sensitivity self-check (opt-in, arXiv:2506.22316): re-judge every
    // decisive calibration item with a cosmetically reordered prompt and
    // measure the fraction whose verdict flips. A single calibration run
    // cannot see this on its own — cosmetic perturbations move scores in
    // judge-specific directions, so the fragility has to be probed directly.
    let mut format_sensitivity_flip_rate = None;
    let mut format_sensitivity_n = 0u64;
    if config.format_sensitivity_check && !calibration_probe_items.is_empty() {
        let mut flips = 0u64;
        for (probe_task, original_verdict_bool) in &calibration_probe_items {
            let probe_prompt = judge_user_prompt(probe_task, true);
            let started = Instant::now();
            let response = model_client.complete(ModelRequest {
                model: &config.model,
                system_prompt: &judge_system_prompt,
                user_prompt: &probe_prompt,
                max_output_units: None,
                temperature: config.temperature,
            })?;
            let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
            total_latency_ms = total_latency_ms.saturating_add(latency_ms);
            if let Some(cost) = response.cost_usd {
                total_cost_usd += cost;
                any_cost_recorded = true;
            }
            let probe_verdict = parse_judge_verdict(&response.output).with_context(|| {
                format!(
                    "format-sensitivity probe for task {:?} returned an unparseable verdict",
                    probe_task.task_id
                )
            })?;
            let flipped = probe_verdict.as_bool() != Some(*original_verdict_bool);
            push_trace_step(
                &mut trace_steps,
                &mut trace_sequence,
                "format_sensitivity_probe",
                &probe_task.task_id,
                serde_json::json!({
                    "cosmetic_reorder": true,
                    "raw_output": response.output,
                    "original_verdict": original_verdict_bool,
                    "flipped": flipped,
                }),
                Some(match probe_verdict {
                    JudgeVerdict::Pass => "pass",
                    JudgeVerdict::Fail => "fail",
                    JudgeVerdict::Unknown => "unknown",
                }),
            );
            if flipped {
                flips += 1;
            }
        }
        format_sensitivity_n = calibration_probe_items.len() as u64;
        format_sensitivity_flip_rate = Some(flips as f64 / format_sensitivity_n as f64);
    }

    calibration_rubric_hashes.sort();
    let calibration_rubric_hash = stable_hash(
        &calibration_rubric_hashes
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let licence_key = judge_licence_key(
        &config.model,
        &system_prompt_hash,
        &calibration_rubric_hash,
        &spec.task,
    );
    // This run's decisive calibration verdicts keyed by task id — the probe
    // set the drift check (backlog 970) compares against a prior run's.
    let current_probe_verdicts: BTreeMap<String, bool> = calibration_probe_items
        .iter()
        .map(|(task, verdict)| (task.task_id.clone(), *verdict))
        .collect();
    let previous_probe_verdicts = config
        .previous_evidence_path
        .as_deref()
        .map(load_previous_probe_verdicts)
        .transpose()?;
    let calibration = build_calibration_record(BuildCalibrationInput {
        judge_id: &config.model,
        judge_verdicts: &calibration_judge,
        expected_verdicts: &calibration_human,
        unknown_count: unknown_calibration,
        generator_id: config.generator_model.as_deref(),
        self_evaluation_bias_risk,
        licence_key: &licence_key,
        task_family: &spec.task,
        format_sensitivity_flip_rate,
        format_sensitivity_n,
        current_probe_verdicts: &current_probe_verdicts,
        previous_probe_verdicts: previous_probe_verdicts.as_ref(),
        drift_checked_at: unix_now_seconds()?,
    });

    let call_count = task_results.len() as u64;
    let judge_stats = JudgeRunStats {
        call_count,
        total_latency_ms,
        mean_latency_ms: if call_count == 0 {
            0.0
        } else {
            total_latency_ms as f64 / call_count as f64
        },
        total_cost_usd: any_cost_recorded.then_some(total_cost_usd),
        unknown_verdict_count: unknown_scored + unknown_calibration,
        failure_rate: if call_count == 0 {
            0.0
        } else {
            (unknown_scored + unknown_calibration) as f64 / call_count as f64
        },
    };

    let score = wilson_score("judge_pass_rate", scored_successes, scored_n);
    let evidence_path = out.join("agentic-judge-run.json");
    write_json(
        &evidence_path,
        &AgenticJudgeEvidence {
            schema_version: "crucible.agentic_judge_evidence.v1",
            spec_id: spec_id(spec, spec_path),
            spec: spec_path.display().to_string(),
            runner: runner.kind,
            provider: config.provider,
            model: config.model.clone(),
            temperature: config.temperature,
            harness: config.harness.clone(),
            tool_allowlist: config.tool_allowlist.clone(),
            system_prompt_hash,
            score: &score,
            totals: PromptTotals {
                tasks: scored_n,
                passed: scored_successes,
                failed: scored_n - scored_successes,
            },
            tasks: &task_results,
            calibration: calibration.as_ref(),
            judge_stats,
        },
    )?;

    // Persist the ordered trace alongside the evidence — same
    // artifact-pointer discipline as `evidence_path`/`spec_path`: a pointer
    // in `artifacts`, not a parallel storage mechanism (backlog 030).
    let trace_path = out.join("agentic-judge-trace.json");
    write_json(
        &trace_path,
        &Trace {
            schema_version: TRACE_SCHEMA.to_string(),
            subject_id: spec_id(spec, spec_path),
            steps: trace_steps,
        },
    )?;

    let mut notes = vec![
        "Executed from a Crucible-authored agentic judge runner with a live model boundary (GraderKind::Agentic, backlog 012)."
            .to_string(),
        format!(
            "Judged {scored_n} candidate task(s) against {:?}/{} rubrics.",
            config.provider, config.model
        ),
    ];
    if unknown_scored > 0 {
        notes.push(format!(
            "{unknown_scored} scored task(s) returned UNKNOWN and were excluded from the score's denominator rather than counted as pass or fail."
        ));
    }
    if self_evaluation_bias_risk {
        notes.push(format!(
            "Self-evaluation bias risk: judge model {} and candidate generator {} share a model family (report §6 mitigation: diversify judges or calibrate against human labels).",
            config.model,
            config.generator_model.as_deref().unwrap_or("?"),
        ));
    }
    notes.extend(canary_notes);
    match &calibration {
        Some(record) if record.unlocked => notes.push(format!(
            "Calibration UNLOCKED: {} paired task(s) vs. the deterministic tier, agreement {:.2}, κ {:.2} (threshold {:.2}), FP rate {:.2}, FN rate {:.2} — this judge's score is trusted. Licence key: {}",
            record.n, record.agreement, record.cohen_kappa, record.unlock_threshold,
            record.false_positive_rate, record.false_negative_rate, record.licence_key
        )),
        Some(record) => notes.push(format!(
            "Calibration LOCKED: {} paired task(s) vs. the deterministic tier, agreement {:.2}, κ {:.2} (threshold {:.2}), FP rate {:.2}, FN rate {:.2} — this judge's score is diagnostic, not trusted. Licence key: {}",
            record.n, record.agreement, record.cohen_kappa, record.unlock_threshold,
            record.false_positive_rate, record.false_negative_rate, record.licence_key
        )),
        None => notes.push(
            "No calibration tasks declared (no task carried a known expected_pass); this judge's score is unlicensed/diagnostic."
                .to_string(),
        ),
    }

    Ok(EvalReport {
        id: spec_id(spec, spec_path),
        title: if spec.task.is_empty() {
            "Agentic judge".to_string()
        } else {
            spec.task.clone()
        },
        score,
        artifacts: vec![
            spec_path.display().to_string(),
            evidence_path.display().to_string(),
            trace_path.display().to_string(),
        ],
        notes,
    })
}

/// Parse a judge response for the `VERDICT: PASS`/`VERDICT: FAIL`/`VERDICT:
/// UNKNOWN` line the reasoning-first judge protocol requires as the FINAL
/// line ([`JUDGE_VERDICT_PROTOCOL`]).
///
/// Tail-anchored: only the last non-empty line is read for the tag. This is
/// deliberate, not incidental — RubricEval (arXiv:2603.25133) found
/// reasoning-before-verdict adds 6.7-9.0 balanced-accuracy points over
/// verdict-first, so the protocol puts the verdict last; a reasoning
/// paragraph is free to discuss the word "verdict" or mention a tag in
/// passing without creating ambiguity, because only the final line's tag is
/// ever taken. A response in the pre-2026-07-06 verdict-first format (tag on
/// the first line, reasoning after) is rejected here, not silently accepted:
/// its final line carries no tag.
///
/// Still refuses to guess: a missing tag, or more than one tag, on that
/// final line is an error, never silently defaulted — `UNKNOWN` is a verdict
/// the judge states explicitly, not a fallback for a response this parser
/// can't read.
fn parse_judge_verdict(output: &str) -> anyhow::Result<JudgeVerdict> {
    let Some(last_line) = output.lines().map(str::trim).rfind(|l| !l.is_empty()) else {
        anyhow::bail!("judge response was empty: {output:?}");
    };
    let upper = last_line.to_uppercase();
    let pass = upper.contains("VERDICT: PASS") || upper.contains("VERDICT:PASS");
    let fail = upper.contains("VERDICT: FAIL") || upper.contains("VERDICT:FAIL");
    let unknown = upper.contains("VERDICT: UNKNOWN") || upper.contains("VERDICT:UNKNOWN");
    match (pass, fail, unknown) {
        (true, false, false) => Ok(JudgeVerdict::Pass),
        (false, true, false) => Ok(JudgeVerdict::Fail),
        (false, false, true) => Ok(JudgeVerdict::Unknown),
        (false, false, false) => {
            anyhow::bail!(
                "judge response's final line carried no VERDICT: PASS/FAIL/UNKNOWN tag — the reasoning-first protocol requires the verdict as the last line: {output:?}"
            )
        }
        _ => {
            anyhow::bail!(
                "judge response's final line had more than one VERDICT tag: {last_line:?}"
            )
        }
    }
}

/// Inputs to [`build_calibration_record`], bundled so the function reads as
/// one decision (assemble the record) rather than eight loose positional
/// arguments.
struct BuildCalibrationInput<'a> {
    judge_id: &'a str,
    judge_verdicts: &'a [bool],
    expected_verdicts: &'a [bool],
    /// Calibration tasks the judge answered `UNKNOWN` on — excluded from
    /// `judge_verdicts`/`expected_verdicts`, reported separately.
    unknown_count: u64,
    generator_id: Option<&'a str>,
    self_evaluation_bias_risk: bool,
    licence_key: &'a str,
    /// The eval's task family ([`EvalSpec::task`]), stamped onto the record
    /// alongside being folded into `licence_key` (backlog 970).
    task_family: &'a str,
    /// Format-sensitivity self-check outputs (see the runner's
    /// `format_sensitivity_check` config flag). `None`/`0` when the check was
    /// not run.
    format_sensitivity_flip_rate: Option<f64>,
    format_sensitivity_n: u64,
    /// This run's decisive calibration verdicts keyed by task id — the probe
    /// set the drift check compares against a prior run's.
    current_probe_verdicts: &'a std::collections::BTreeMap<String, bool>,
    /// A prior run's calibration verdicts over the same probe set, keyed by
    /// task id (backlog 970's drift check, `AgenticJudgeConfig::previous_evidence_path`).
    /// `None` when no prior run was supplied for comparison.
    previous_probe_verdicts: Option<&'a std::collections::BTreeMap<String, bool>>,
    /// Caller-supplied Unix-seconds timestamp for the drift comparison.
    /// Ignored (the record's `drift_checked_at` stays `None`) when
    /// `previous_probe_verdicts` is `None`.
    drift_checked_at: i64,
}

/// Build the judge's [`CalibrationRecord`] (backlog 012) from the paired
/// (judge verdict, deterministic/expected verdict) vectors accumulated over
/// every task in the run that carried a known `expected_pass` — every
/// calibration probe, not only the judge-gaming canary. `None` when the run
/// declared no *decisive* calibration tasks at all (an all-`UNKNOWN`
/// calibration set counts as none): an unmeasured judge has no record to
/// report, not a fabricated `unlocked: true`.
///
/// Reuses `crucible_core::measure`'s [`agreement`]/[`cohen_kappa`] kernels (the
/// same ones the leaderboard's noise-floor discipline uses) rather than
/// recomputing agreement here — this function only assembles the record from
/// their outputs, per [`CalibrationRecord`]'s own "records, does not compute"
/// contract. `unlocked` gates on raw [`agreement`] against
/// [`CALIBRATION_AGREEMENT_THRESHOLD`] (backlog 012's oracle: "clears the
/// configured agreement threshold"); κ is recorded for audit but does not
/// itself gate. A degenerate κ (all-one-label calibration set) records as
/// `0.0` — descriptive metadata on the record, never the gate input, so an
/// undefined κ cannot silently unlock a judge.
fn build_calibration_record(input: BuildCalibrationInput<'_>) -> Option<CalibrationRecord> {
    let BuildCalibrationInput {
        judge_id,
        judge_verdicts,
        expected_verdicts,
        unknown_count,
        generator_id,
        self_evaluation_bias_risk,
        licence_key,
        task_family,
        format_sensitivity_flip_rate,
        format_sensitivity_n,
        current_probe_verdicts,
        previous_probe_verdicts,
        drift_checked_at,
    } = input;
    if judge_verdicts.is_empty() {
        return None;
    }
    let observed_agreement = agreement(judge_verdicts, expected_verdicts).unwrap_or(0.0);
    let kappa = cohen_kappa(judge_verdicts, expected_verdicts).unwrap_or(0.0);
    let mut confusion = ConfusionMatrix::default();
    for (&judge, &expected) in judge_verdicts.iter().zip(expected_verdicts.iter()) {
        match (judge, expected) {
            (true, true) => confusion.true_positive += 1,
            (true, false) => confusion.false_positive += 1,
            (false, true) => confusion.false_negative += 1,
            (false, false) => confusion.true_negative += 1,
        }
    }
    let false_positive_rate = confusion.false_positive_rate();
    let false_negative_rate = confusion.false_negative_rate();
    let fail_class_precision = confusion.fail_precision();
    let fail_class_recall = confusion.fail_recall();
    let (drift_flip_rate, drift_probe_n, drift_checked_at) = match previous_probe_verdicts {
        Some(previous) => match probe_drift(previous, current_probe_verdicts) {
            Some((rate, n)) => (Some(rate), n, Some(drift_checked_at)),
            None => (None, 0, None),
        },
        None => (None, 0, None),
    };
    Some(CalibrationRecord {
        schema_version: CALIBRATION_RECORD_SCHEMA.to_string(),
        judge_id: judge_id.to_string(),
        n: judge_verdicts.len() as u64,
        agreement: observed_agreement,
        cohen_kappa: kappa,
        confusion,
        false_positive_rate,
        false_negative_rate,
        unknown_count,
        generator_id: generator_id.map(str::to_string),
        self_evaluation_bias_risk,
        unlock_threshold: CALIBRATION_AGREEMENT_THRESHOLD,
        unlocked: observed_agreement >= CALIBRATION_AGREEMENT_THRESHOLD,
        licence_key: licence_key.to_string(),
        format_sensitivity_flip_rate,
        format_sensitivity_n,
        fail_class_precision,
        fail_class_recall,
        task_family: task_family.to_string(),
        drift_flip_rate,
        drift_probe_n,
        drift_checked_at,
    })
}

/// Compute pass^k task consistency (backlog 015) over a batch of graded
/// trials: group by `task_id`, require every task to share the same trial
/// count `k ≥ 2` (otherwise there is no single `k` to report — returns
/// `None`), and Wilson-score the fraction of tasks where *every* trial fully
/// matched the key (zero missed, zero false positives).
fn compute_pass_k(task_results: &[TaskResult]) -> Option<PassKScore> {
    let mut by_task: std::collections::BTreeMap<&str, Vec<&TaskResult>> =
        std::collections::BTreeMap::new();
    for result in task_results {
        by_task
            .entry(result.task_id.as_str())
            .or_default()
            .push(result);
    }

    let mut k: Option<u64> = None;
    for trials in by_task.values() {
        let n = trials.len() as u64;
        match k {
            None => k = Some(n),
            Some(existing) if existing != n => return None,
            _ => {}
        }
    }
    let k = k?;
    if k < 2 {
        return None;
    }

    let n_tasks = by_task.len() as u64;
    let n_tasks_all_passed = by_task
        .values()
        .filter(|trials| {
            trials
                .iter()
                .all(|t| t.missed == 0 && t.false_positives == 0)
        })
        .count() as u64;

    Some(PassKScore {
        k,
        score: wilson_score("pass_k_task_consistency", n_tasks_all_passed, n_tasks),
    })
}

fn grade_key_recall_task(findings: &[KeyFinding], expected: &ExpectedKey) -> KeyRecallTaskScore {
    // The span-aware match (file + category + line-in-span + severity floor)
    // lives in crucible-core (backlog 013): Threshold/Daedalus share this
    // scorer by construction instead of by prose parity.
    let grade = crucible_core::score_against_expected_key(findings, expected);
    let matched = grade.matched_ids.len() as u64;
    let missed = grade.missed_ids.len() as u64;
    let false_positives = grade.false_positives;
    KeyRecallTaskScore {
        matched,
        missed,
        false_positives,
        expected_defects: matched + missed,
        grade,
    }
}

fn spec_id(spec: &EvalSpec, spec_path: &Path) -> String {
    if !spec.id.trim().is_empty() {
        return spec.id.clone();
    }
    spec_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("declared-eval")
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSpecPath {
    pub path: PathBuf,
    pub declared: String,
    pub alias: Option<&'static str>,
}

pub(crate) fn resolve_spec_path_with_alias(spec_path: &Path, raw: &str) -> ResolvedSpecPath {
    let path = PathBuf::from(raw);
    let declared = if path.is_absolute() {
        path
    } else {
        spec_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    };
    if declared.exists() {
        return ResolvedSpecPath {
            path: canonicalize_existing(&declared),
            declared: raw.to_string(),
            alias: None,
        };
    }
    if !Path::new(raw).is_absolute() {
        if let Some(alias_raw) = daedalus_to_threshold_raw(raw) {
            let alias_path = spec_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(alias_raw);
            if alias_path.exists() {
                return ResolvedSpecPath {
                    path: canonicalize_existing(&alias_path),
                    declared: raw.to_string(),
                    alias: Some("daedalus_to_threshold"),
                };
            }
        }
    }
    ResolvedSpecPath {
        path: declared,
        declared: raw.to_string(),
        alias: None,
    }
}

fn canonicalize_existing(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_spec_path(spec_path: &Path, raw: &str) -> PathBuf {
    resolve_spec_path_with_alias(spec_path, raw).path
}

fn daedalus_to_threshold_raw(raw: &str) -> Option<PathBuf> {
    let mut replaced = false;
    let mut out = PathBuf::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(name) if name == "daedalus" && !replaced => {
                out.push("threshold");
                replaced = true;
            }
            other => out.push(other.as_os_str()),
        }
    }
    if replaced {
        Some(out)
    } else {
        None
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(value)
        .with_context(|| format!("serializing {}", path.display()))?;
    std::fs::write(path, format!("{json}\n")).with_context(|| format!("writing {}", path.display()))
}

trait ModelClient {
    fn complete(&self, request: ModelRequest<'_>) -> anyhow::Result<ModelResponse>;
}

struct OpenRouterClient {
    api_key: String,
    http: reqwest::blocking::Client,
}

impl OpenRouterClient {
    fn from_config(config: &PromptModelConfig) -> anyhow::Result<Self> {
        Self::from_credential_env(&config.credential_env)
    }

    fn from_credential_env(credential_env: &str) -> anyhow::Result<Self> {
        let api_key = std::env::var(credential_env).with_context(|| {
            format!("{credential_env} is not set; this runner requires a BYOK OpenRouter key")
        })?;
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("building OpenRouter HTTP client")?;
        Ok(Self { api_key, http })
    }
}

impl ModelClient for OpenRouterClient {
    fn complete(&self, request: ModelRequest<'_>) -> anyhow::Result<ModelResponse> {
        #[cfg(debug_assertions)]
        if let Ok(output) = std::env::var("CRUCIBLE_OPENROUTER_FIXTURE_OUTPUT") {
            return Ok(ModelResponse {
                output,
                response_id: Some(format!("fixture:{}", request.model)),
                response_model: Some(request.model.to_string()),
                input_units: Some(1),
                output_units: Some(1),
                total_units: Some(2),
                cost_usd: Some(0.0),
            });
        }

        let body = ChatCompletionRequest {
            model: request.model,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: request.system_prompt,
                },
                ChatMessage {
                    role: "user",
                    content: request.user_prompt,
                },
            ],
            max_output_units: request.max_output_units,
            temperature: request.temperature,
        };
        let response = self
            .http
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://github.com/misty-step/crucible")
            .header("X-OpenRouter-Title", "Crucible")
            .json(&body)
            .send()
            .context("sending OpenRouter chat completion request")?;

        let status = response.status();
        if !status.is_success() {
            let text = response
                .text()
                .unwrap_or_else(|_| "<failed to read response body>".to_string());
            anyhow::bail!(
                "OpenRouter chat completion failed with status {}: {}",
                status,
                truncate_for_error(&text)
            );
        }

        let response: ChatCompletionResponse = response
            .json()
            .context("parsing OpenRouter chat completion response")?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .context("OpenRouter response had no choices")?;
        let output = chat_content_to_string(choice.message.content)
            .context("OpenRouter response choice had no text content")?;
        Ok(ModelResponse {
            output,
            response_id: response.id,
            response_model: response.model,
            input_units: response.usage.as_ref().and_then(|usage| usage.input_units),
            output_units: response.usage.as_ref().and_then(|usage| usage.output_units),
            total_units: response.usage.as_ref().and_then(|usage| usage.total_units),
            cost_usd: response.usage.and_then(|usage| usage.cost),
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct ModelRequest<'a> {
    model: &'a str,
    system_prompt: &'a str,
    user_prompt: &'a str,
    max_output_units: Option<u32>,
    temperature: Option<u32>,
}

#[derive(Debug)]
struct ModelResponse {
    output: String,
    response_id: Option<String>,
    response_model: Option<String>,
    input_units: Option<u64>,
    output_units: Option<u64>,
    total_units: Option<u64>,
    cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    #[serde(rename = "max_tokens", skip_serializing_if = "Option::is_none")]
    max_output_units: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<u32>,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    #[serde(rename = "prompt_tokens", default)]
    input_units: Option<u64>,
    #[serde(default)]
    #[serde(rename = "completion_tokens")]
    output_units: Option<u64>,
    #[serde(default)]
    #[serde(rename = "total_tokens")]
    total_units: Option<u64>,
    #[serde(default)]
    cost: Option<f64>,
}

fn chat_content_to_string(content: serde_json::Value) -> Option<String> {
    match content {
        serde_json::Value::String(text) => Some(text),
        serde_json::Value::Null => Some(String::new()),
        serde_json::Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                if let Some(text) = part.get("text").and_then(serde_json::Value::as_str) {
                    out.push_str(text);
                }
            }
            Some(out)
        }
        _ => None,
    }
}

/// Grade a model response against a declared rubric. Returns `Err` for a
/// `Regex` variant whose `pattern` fails to compile — a malformed pattern is
/// a spec error surfaced at grading time here (and, before any model call
/// runs, at `crucible validate`/preflight time — see `check_prompt_regexes`),
/// never a panic.
fn prompt_expectation_passes(
    output: &str,
    expectation: &PromptExpectation,
) -> anyhow::Result<bool> {
    Ok(match expectation {
        PromptExpectation::Exact { value } => output.trim() == value.trim(),
        PromptExpectation::Contains { value } => output.contains(value),
        PromptExpectation::CaseInsensitiveContains { value } => {
            output.to_lowercase().contains(&value.to_lowercase())
        }
        PromptExpectation::Regex { pattern } => {
            compile_expectation_regex(pattern)?.is_match(output)
        }
        PromptExpectation::StrictJson { value } => {
            match serde_json::from_str::<serde_json::Value>(output.trim()) {
                Ok(parsed) => parsed == *value,
                Err(_) => false,
            }
        }
        PromptExpectation::PythonUnitTest {
            test_source,
            timeout_ms,
        } => python_unit_test_passes(output, test_source, *timeout_ms)?,
    })
}

fn python_unit_test_passes(
    solution_source: &str,
    test_source: &str,
    timeout_ms: Option<u64>,
) -> anyhow::Result<bool> {
    let root = unique_temp_dir("crucible-python-unit")?;
    let result = run_python_unit_test_in(&root, solution_source, test_source, timeout_ms);
    let _ = std::fs::remove_dir_all(&root);
    result
}

fn run_python_unit_test_in(
    root: &Path,
    solution_source: &str,
    test_source: &str,
    timeout_ms: Option<u64>,
) -> anyhow::Result<bool> {
    std::fs::create_dir_all(root)
        .with_context(|| format!("creating python unit test directory {}", root.display()))?;
    std::fs::write(root.join("solution.py"), solution_source)
        .with_context(|| format!("writing {}", root.join("solution.py").display()))?;
    std::fs::write(root.join("test_solution.py"), test_source)
        .with_context(|| format!("writing {}", root.join("test_solution.py").display()))?;

    let mut child = Command::new("python3")
        .arg("-I")
        .arg("-c")
        .arg("import sys; sys.path.insert(0, '.'); exec(open('test_solution.py', encoding='utf-8').read())")
        .current_dir(root)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawning python3 for python_unit_test expectation")?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.unwrap_or(3000));
    loop {
        if let Some(status) = child
            .try_wait()
            .context("checking python unit test status")?
        {
            return Ok(status.success());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn unique_temp_dir(prefix: &str) -> anyhow::Result<PathBuf> {
    let base = std::env::temp_dir();
    for attempt in 0..100u32 {
        let path = base.join(format!(
            "{prefix}-{}-{}-{attempt}",
            std::process::id(),
            temp_dir_nonce()?
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("creating temporary directory {}", path.display()))
            }
        }
    }
    anyhow::bail!("could not allocate a unique temporary directory for {prefix}")
}

fn temp_dir_nonce() -> anyhow::Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos())
}

/// Current Unix-seconds timestamp — the only clock read in the drift-check
/// path; [`crucible_core::CalibrationRecord::drift_checked_at`] is otherwise
/// entirely caller-supplied.
fn unix_now_seconds() -> anyhow::Result<i64> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs();
    Ok(secs as i64)
}

/// Load a prior run's decisive calibration verdicts from its
/// `agentic-judge-run.json` evidence file, keyed by task id — the baseline
/// [`probe_drift`] compares this run's probe set against
/// (`AgenticJudgeConfig::previous_evidence_path`). Only tasks that carried a
/// known `expected_pass` in the prior run count as probe tasks, and only a
/// decisive (`pass`/`fail`) verdict is recorded — an `unknown` verdict is
/// excluded, matching how the current run's own probe set is built.
fn load_previous_probe_verdicts(path: &Path) -> anyhow::Result<BTreeMap<String, bool>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading previous calibration evidence {}", path.display()))?;
    let evidence: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing previous calibration evidence {}", path.display()))?;
    let tasks = evidence
        .get("tasks")
        .and_then(|v| v.as_array())
        .with_context(|| {
            format!(
                "previous calibration evidence {} has no \"tasks\" array",
                path.display()
            )
        })?;
    let mut verdicts = BTreeMap::new();
    for task in tasks {
        if task
            .get("expected_pass")
            .and_then(|v| v.as_bool())
            .is_none()
        {
            continue; // not a calibration probe in the prior run
        }
        let Some(task_id) = task.get("task_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let verdict = match task.get("verdict").and_then(|v| v.as_str()) {
            Some("pass") => true,
            Some("fail") => false,
            _ => continue, // unknown or missing — not decisive, excluded
        };
        verdicts.insert(task_id.to_string(), verdict);
    }
    Ok(verdicts)
}

/// Compile a `Regex` expectation's pattern, with an error that names the
/// pattern rather than surfacing the raw `regex` crate error alone.
fn compile_expectation_regex(pattern: &str) -> anyhow::Result<regex::Regex> {
    regex::Regex::new(pattern)
        .with_context(|| format!("prompt expectation regex {pattern:?} failed to compile"))
}

fn expectation_kind(expectation: &PromptExpectation) -> &'static str {
    match expectation {
        PromptExpectation::Exact { .. } => "exact",
        PromptExpectation::Contains { .. } => "contains",
        PromptExpectation::CaseInsensitiveContains { .. } => "case_insensitive_contains",
        PromptExpectation::Regex { .. } => "regex",
        PromptExpectation::StrictJson { .. } => "strict_json",
        PromptExpectation::PythonUnitTest { .. } => "python_unit_test",
    }
}

fn expectation_value(expectation: &PromptExpectation) -> String {
    match expectation {
        PromptExpectation::Exact { value }
        | PromptExpectation::Contains { value }
        | PromptExpectation::CaseInsensitiveContains { value } => value.clone(),
        PromptExpectation::Regex { pattern } => pattern.clone(),
        PromptExpectation::StrictJson { value } => value.to_string(),
        PromptExpectation::PythonUnitTest {
            test_source,
            timeout_ms,
        } => format!("timeout={}:{}", timeout_ms.unwrap_or(3000), test_source),
    }
}

/// Current wall-clock time as an RFC 3339 string, for a [`TraceStep`]'s
/// `timestamp`. `Trace`/`TraceStep` themselves never read the clock (the
/// same reproducibility discipline as `Provenance`/`EvaluationCard`) — only
/// the runner, which already owns the run's non-reproducible side effects
/// (the live model call), supplies it.
fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

/// Append one ordered [`TraceStep`] to `steps`, stamping it with the next
/// `sequence` value and the current time. Centralizes the "sequence is
/// gapless and ascending from 0" invariant the trace consumers rely on
/// (backlog 030) so no call site has to track the counter by hand.
fn push_trace_step(
    steps: &mut Vec<TraceStep>,
    sequence: &mut u64,
    kind: &str,
    label: &str,
    detail: serde_json::Value,
    outcome: Option<&str>,
) {
    steps.push(TraceStep {
        sequence: *sequence,
        timestamp: now_rfc3339(),
        kind: kind.to_string(),
        label: label.to_string(),
        detail,
        outcome: outcome.map(str::to_string),
    });
    *sequence += 1;
}

fn stable_hash(parts: &[&str]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn truncate_for_error(text: &str) -> String {
    const LIMIT: usize = 320;
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= LIMIT {
        compact
    } else {
        format!("{}...", &compact[..LIMIT])
    }
}

fn load_cerberus_receipt_bundle(path: &Path) -> anyhow::Result<CerberusReviewReceiptBundle> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading Cerberus receipt bundle {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "parsing {} as a Cerberus ReviewReceiptBundle",
            path.display()
        )
    })
}

fn validate_cerberus_receipt(
    receipt: &CerberusReviewReceiptBundle,
    path: &Path,
) -> anyhow::Result<()> {
    if receipt.schema_version != "cerberus.review_receipt_bundle.v1" {
        anyhow::bail!(
            "{} has unsupported schema_version {:?}",
            path.display(),
            receipt.schema_version
        );
    }
    if receipt.validation.status != "passed" {
        anyhow::bail!(
            "{} is not trusted for grading: validation.status={:?}",
            path.display(),
            receipt.validation.status
        );
    }
    Ok(())
}

fn receipt_artifact_uri_matches(
    receipt_uri: &str,
    declared_artifact: &str,
    resolved: &Path,
) -> bool {
    if receipt_uri == declared_artifact || receipt_uri == resolved.display().to_string() {
        return true;
    }
    let receipt_path = Path::new(receipt_uri);
    if receipt_path.is_absolute() {
        receipt_path == resolved
    } else {
        resolved.ends_with(receipt_path)
    }
}

#[derive(Debug, Deserialize)]
struct DaedalusTrial {
    run_id: String,
    #[serde(default)]
    arena_id: Option<String>,
    #[serde(default)]
    arena_version: Option<String>,
    task_id: String,
    #[serde(default)]
    trial: Option<u64>,
    candidate_id: String,
    #[serde(default)]
    candidate_kind: Option<String>,
    #[serde(default)]
    reward: Option<f64>,
    #[serde(default)]
    recall: Option<f64>,
    #[serde(default)]
    false_positives: Option<u64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    scorer_error: Option<String>,
    #[serde(default)]
    findings: Option<Vec<KeyFinding>>,
    #[serde(default)]
    artifacts: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CerberusReviewReceiptBundle {
    schema_version: String,
    artifact_id: String,
    harness: String,
    #[serde(default)]
    model: Option<String>,
    artifact_uri: String,
    validation: CerberusReceiptValidation,
}

#[derive(Debug, Deserialize)]
struct CerberusReceiptValidation {
    status: String,
    #[serde(default)]
    trusted_for_posting: bool,
}

#[derive(Debug)]
struct KeyRecallTaskScore {
    matched: u64,
    missed: u64,
    false_positives: u64,
    expected_defects: u64,
    grade: crucible_core::SpanGrade,
}

#[derive(Debug, Serialize)]
struct SpecRunEvidence<'a> {
    schema_version: &'static str,
    spec_id: String,
    spec: String,
    runner: RunnerKind,
    corpus: CorpusEvidence,
    score: &'a Score,
    totals: Totals,
    tasks: &'a [TaskResult],
    /// pass^k task-consistency (backlog 015): `None` unless every task in this
    /// run shares the same trial count `k ≥ 2` — an uneven repetition count
    /// has no single `k` to report a consistency rate for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pass_k: Option<&'a PassKScore>,
}

/// pass^k over one run's tasks: `k` repeated trials per task, scored as
/// PASSED (`k` for `k`) only when *every* trial fully matched the adjudicated
/// key (zero missed defects, zero false positives) — the same "task counts as
/// solved iff every trial earned full reward" bar
/// [`crucible_core::Leaderboard`]'s `solve_rate` uses. `score` is a Wilson
/// proportion over tasks (the independence unit — trials of the same task are
/// correlated), not trials.
#[derive(Debug, Serialize)]
struct PassKScore {
    k: u64,
    score: Score,
}

#[derive(Debug, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
enum CorpusEvidence {
    DaedalusTrials {
        arena_dir: String,
        trials_jsonl: String,
        declared_arena_dir: String,
        declared_trials_jsonl: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        arena_dir_alias: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        trials_jsonl_alias: Option<String>,
        candidate_id: String,
        selected_tasks: Vec<String>,
    },
    CerberusReceiptBundles {
        candidate_id: String,
        tasks: Vec<CerberusReceiptEvidence>,
    },
}

#[derive(Debug, Serialize)]
struct PromptRunEvidence<'a> {
    schema_version: &'static str,
    spec_id: String,
    spec: String,
    runner: RunnerKind,
    provider: ModelProvider,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_units: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    harness: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_allowlist: Vec<String>,
    system_prompt_hash: String,
    score: &'a Score,
    totals: PromptTotals,
    tasks: &'a [PromptTaskResult],
}

#[derive(Debug, Serialize)]
struct PromptTotals {
    tasks: u64,
    passed: u64,
    failed: u64,
}

#[derive(Debug, Serialize)]
struct PromptTaskResult {
    task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_file: Option<String>,
    prompt_hash: String,
    rubric_hash: String,
    expectation: PromptExpectation,
    passed: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tracked_results: Vec<TrackedCheckResult>,
    output: String,
    latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_id: Option<String>,
    requested_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_model: Option<String>,
    #[serde(rename = "prompt_tokens", skip_serializing_if = "Option::is_none")]
    input_units: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "completion_tokens")]
    output_units: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "total_tokens")]
    total_units: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
struct TrackedCheckResult {
    id: String,
    passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackedFailure {
    pub evidence_path: String,
    pub task_id: String,
    pub check_id: String,
}

/// Read tracked-check failures from a completed run's prompt evidence. This is
/// intentionally post-run: strict tracked mode promotes diagnostic checks into
/// a process exit code without changing the score or persisted record.
pub(crate) fn tracked_failures(report: &RunReport) -> anyhow::Result<Vec<TrackedFailure>> {
    let mut failures = Vec::new();
    for eval in &report.evals {
        for artifact in &eval.artifacts {
            if !artifact.ends_with("prompt-run.json") {
                continue;
            }
            let bytes = std::fs::read(artifact)
                .with_context(|| format!("reading prompt evidence {artifact}"))?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing prompt evidence {artifact}"))?;
            let tasks = value
                .get("tasks")
                .and_then(serde_json::Value::as_array)
                .with_context(|| format!("{artifact} is prompt evidence without a tasks array"))?;
            for task in tasks {
                let task_id = task
                    .get("task_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("<unknown>")
                    .to_string();
                let Some(tracked_results) = task
                    .get("tracked_results")
                    .and_then(serde_json::Value::as_array)
                else {
                    continue;
                };
                for check in tracked_results {
                    if check
                        .get("passed")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true)
                    {
                        continue;
                    }
                    failures.push(TrackedFailure {
                        evidence_path: artifact.clone(),
                        task_id: task_id.clone(),
                        check_id: check
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("<unknown>")
                            .to_string(),
                    });
                }
            }
        }
    }
    Ok(failures)
}

pub(crate) fn format_tracked_failures(failures: &[TrackedFailure]) -> String {
    failures
        .iter()
        .map(|failure| {
            format!(
                "{}:{} ({})",
                failure.task_id, failure.check_id, failure.evidence_path
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Serialize)]
struct AgenticJudgeEvidence<'a> {
    schema_version: &'static str,
    spec_id: String,
    spec: String,
    runner: RunnerKind,
    provider: ModelProvider,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    harness: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_allowlist: Vec<String>,
    system_prompt_hash: String,
    score: &'a Score,
    totals: PromptTotals,
    tasks: &'a [AgenticJudgeTaskResult],
    /// The judge's calibration against the deterministic tier on this run's
    /// calibration tasks (backlog 012). `None` when the run declared none.
    #[serde(skip_serializing_if = "Option::is_none")]
    calibration: Option<&'a CalibrationRecord>,
    /// Judge-specific cost/latency/failure-rate for this run (report §6 item
    /// 11), distinct from any candidate-side generation cost — this runner
    /// judges authored candidate strings, so every call here is a judge call.
    judge_stats: JudgeRunStats,
}

/// Judge-specific aggregate stats for one run: total/mean latency, total
/// cost (when the provider reported one), and how often the judge answered
/// `UNKNOWN` rather than a decisive verdict — the judge's own failure-to-decide
/// rate, distinguishable from any candidate-generation cost/latency tracked
/// elsewhere.
#[derive(Debug, Serialize)]
struct JudgeRunStats {
    call_count: u64,
    total_latency_ms: u64,
    mean_latency_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_cost_usd: Option<f64>,
    unknown_verdict_count: u64,
    failure_rate: f64,
}

#[derive(Debug, Serialize)]
struct AgenticJudgeTaskResult {
    task_id: String,
    prompt_hash: String,
    rubric_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_pass: Option<bool>,
    /// The judge's verdict, tri-state: `"pass"`, `"fail"`, or `"unknown"`
    /// (report §6 item 8). The authoritative field — `passed` below is a
    /// bool mirror kept only for the shared prompt/judge evidence ingestion
    /// path in `run_store.rs`.
    verdict: &'static str,
    passed: bool,
    output: String,
    latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_id: Option<String>,
    requested_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_model: Option<String>,
    #[serde(rename = "prompt_tokens", skip_serializing_if = "Option::is_none")]
    input_units: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "completion_tokens")]
    output_units: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "total_tokens")]
    total_units: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
}

#[derive(Debug, Serialize)]
struct CerberusReceiptEvidence {
    task_id: String,
    artifact: String,
    receipt_bundle: String,
    receipt_artifact_uri: String,
    artifact_uri_matches: bool,
    harness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    validation_status: String,
    trusted_for_posting: bool,
}

#[derive(Debug, Serialize)]
struct Totals {
    trials: u64,
    matched: u64,
    expected_defects: u64,
    disputed: u64,
    recoverable_misses: u64,
}

#[derive(Debug, Serialize)]
struct TaskResult {
    task_id: String,
    run_id: String,
    trial: Option<u64>,
    candidate_id: String,
    candidate_kind: Option<String>,
    arena_id: Option<String>,
    arena_version: Option<String>,
    key: String,
    findings: usize,
    #[serde(skip_serializing_if = "is_zero_usize")]
    dropped_invalid: usize,
    matched: u64,
    matched_ids: Vec<String>,
    missed: u64,
    missed_ids: Vec<String>,
    disputed: u64,
    false_positives: u64,
    recoverable_misses: u64,
    expected_defects: u64,
    daedalus_reward: Option<f64>,
    daedalus_recall: Option<f64>,
    daedalus_false_positives: Option<u64>,
    error: Option<String>,
    scorer_error: Option<String>,
    artifacts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt_bundle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt_harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt_validation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt_trusted_for_posting: Option<bool>,
}

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_for_summary_leaves_short_text_untouched() {
        assert_eq!(truncate_for_summary("hello harbor", 2000), "hello harbor");
    }

    #[test]
    fn truncate_for_summary_truncates_and_marks_long_text() {
        let long = "a".repeat(50);
        let truncated = truncate_for_summary(&long, 10);
        assert_eq!(truncated, format!("{}\n...[truncated]", "a".repeat(10)));
    }

    #[test]
    fn derive_harbor_outcome_reads_full_reward_as_passed() {
        let result_json = serde_json::json!({
            "exception_info": null,
            "verifier_result": { "rewards": { "reward": 1.0 } }
        });
        let outcome = derive_harbor_outcome(&result_json, "crucible-smoke").unwrap();
        assert!(outcome.passed);
        assert_eq!(outcome.reward, 1.0);
        assert!(outcome.exception.is_none());
        assert_eq!(outcome.reward_breakdown, serde_json::json!({"reward": 1.0}));
    }

    #[test]
    fn derive_harbor_outcome_partial_reward_is_not_a_pass() {
        let result_json = serde_json::json!({
            "exception_info": null,
            "verifier_result": { "rewards": { "reward": 0.5 } }
        });
        let outcome = derive_harbor_outcome(&result_json, "partial-task").unwrap();
        assert!(!outcome.passed);
        assert_eq!(outcome.reward, 0.5);
    }

    #[test]
    fn derive_harbor_outcome_exception_always_fails_regardless_of_reward() {
        // Even if Harbor still reported a reward alongside an exception, a
        // non-null exception_info is an unconditional fail (the agent or
        // environment crashed; the reward, if any, isn't trustworthy).
        let result_json = serde_json::json!({
            "exception_info": {"exception_type": "TimeoutError", "exception_message": "agent timed out"},
            "verifier_result": { "rewards": { "reward": 1.0 } }
        });
        let outcome = derive_harbor_outcome(&result_json, "wedged-task").unwrap();
        assert!(!outcome.passed);
        assert_eq!(outcome.reward, 0.0);
        assert!(outcome.exception.is_some());
    }

    #[test]
    fn derive_harbor_outcome_falls_back_to_sole_reward_when_unnamed_reward() {
        // Real fixture shape names the key "reward", but a task that names
        // its single reward differently should still resolve unambiguously.
        let result_json = serde_json::json!({
            "exception_info": null,
            "verifier_result": { "rewards": { "custom_score": 1.0 } }
        });
        let outcome = derive_harbor_outcome(&result_json, "custom-task").unwrap();
        assert!(outcome.passed);
        assert_eq!(outcome.reward, 1.0);
    }

    #[test]
    fn derive_harbor_outcome_refuses_ambiguous_multi_reward_without_primary_key() {
        let result_json = serde_json::json!({
            "exception_info": null,
            "verifier_result": { "rewards": { "speed": 0.9, "correctness": 1.0 } }
        });
        let err = derive_harbor_outcome(&result_json, "multi-reward-task").unwrap_err();
        assert!(
            err.to_string().contains("multi-reward-task"),
            "error should name the task: {err}"
        );
    }

    #[test]
    fn derive_harbor_outcome_refuses_when_rewards_map_is_empty() {
        let result_json = serde_json::json!({
            "exception_info": null,
            "verifier_result": { "rewards": {} }
        });
        let err = derive_harbor_outcome(&result_json, "empty-rewards-task").unwrap_err();
        assert!(err.to_string().contains("empty-rewards-task"));
    }

    #[test]
    fn require_under_home_accepts_a_path_under_home() {
        let home = std::env::var("HOME").expect("HOME must be set to run this test");
        let under_home = Path::new(&home).join("crucible-harbor-test-marker-that-need-not-exist");
        // require_under_home only checks path containment via prefix
        // comparison against the canonicalized $HOME, not existence.
        assert!(require_under_home(&under_home).is_ok());
    }

    #[test]
    fn require_under_home_refuses_a_path_outside_home() {
        let err = require_under_home(Path::new("/etc/definitely-not-under-home")).unwrap_err();
        assert!(err.to_string().contains("$HOME") || err.to_string().contains("Colima"));
    }

    #[test]
    fn read_harbor_trial_result_reads_the_single_trial_subdirectory() {
        let dir = unique_temp_dir("crucible-harbor-trial-test").expect("temp dir");
        let trial_dir = dir.join("crucible-smoke__abc123");
        std::fs::create_dir_all(&trial_dir).unwrap();
        std::fs::write(
            trial_dir.join("result.json"),
            serde_json::json!({"task_name": "misty-step/crucible-smoke"}).to_string(),
        )
        .unwrap();

        let result = read_harbor_trial_result(&dir).unwrap();
        assert_eq!(result.trial_dir, trial_dir);
        assert_eq!(
            result
                .result_json
                .get("task_name")
                .and_then(serde_json::Value::as_str),
            Some("misty-step/crucible-smoke")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_harbor_trial_result_refuses_when_no_trial_subdirectory_exists() {
        let dir = unique_temp_dir("crucible-harbor-empty-job-test").expect("temp dir");
        let err = read_harbor_trial_result(&dir).unwrap_err();
        assert!(err.to_string().contains("no trial subdirectory"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_harbor_trial_result_refuses_when_multiple_trial_subdirectories_exist() {
        let dir = unique_temp_dir("crucible-harbor-multi-trial-test").expect("temp dir");
        for name in ["trial-a", "trial-b"] {
            let trial_dir = dir.join(name);
            std::fs::create_dir_all(&trial_dir).unwrap();
            std::fs::write(trial_dir.join("result.json"), "{}").unwrap();
        }
        let err = read_harbor_trial_result(&dir).unwrap_err();
        assert!(err.to_string().contains("exactly one trial subdirectory"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- trial isolation (docs/AGENTS.md "Trial isolation", crucible-975) ----
    //
    // These are the gate-level leakage probes the isolation contract
    // requires: they run against the REAL `prepare_harbor_job_dir` /
    // `harbor_job_dir` production code paths `run_one_harbor_task` uses, not
    // a parallel test-only implementation, and need no live `harbor`/Docker
    // install (that's why `read_harbor_trial_result_*` above and these tests
    // never spawn the `harbor` CLI: this file's existing pattern is to prove
    // the directory/parsing contract Crucible itself owns without depending
    // on the external tool CI does not install).

    #[test]
    fn harbor_job_directory_clears_prior_trial_artifacts_before_reuse() {
        let jobs_root = unique_temp_dir("crucible-harbor-isolation-reuse").expect("temp dir");

        let job_dir = prepare_harbor_job_dir(&jobs_root, "task-a").expect("prepare job dir");
        std::fs::create_dir_all(&job_dir).unwrap();
        let leaked_marker = job_dir.join("prior-trial-secret.txt");
        std::fs::write(&leaked_marker, "leaked-from-prior-trial").unwrap();
        assert!(
            leaked_marker.exists(),
            "sanity: the simulated prior-trial artifact was actually written"
        );

        // The probe: prepare the SAME task id's job directory again, as a
        // second trial would. If isolation ever regressed (e.g. the clearing
        // step were removed), the leaked marker from the "prior trial" above
        // would still be readable here — this assertion is the leakage
        // probe, and it fails on any cross-trial visibility.
        let job_dir_again =
            prepare_harbor_job_dir(&jobs_root, "task-a").expect("prepare job dir again");
        assert_eq!(job_dir_again, job_dir, "same task id reuses the same slot");
        assert!(
            !leaked_marker.exists(),
            "a prior trial's artifact must not be visible to the next trial reusing this slot"
        );
        assert!(
            !job_dir_again.exists() || std::fs::read_dir(&job_dir_again).unwrap().next().is_none(),
            "the prepared job directory is either absent or empty, never pre-populated"
        );

        let _ = std::fs::remove_dir_all(&jobs_root);
    }

    #[test]
    fn harbor_job_directories_are_disjoint_across_task_ids() {
        let jobs_root = unique_temp_dir("crucible-harbor-isolation-disjoint").expect("temp dir");

        let job_dir_a = prepare_harbor_job_dir(&jobs_root, "task-a").expect("prepare task-a");
        std::fs::create_dir_all(&job_dir_a).unwrap();
        let secret_a = job_dir_a.join("task-a-secret.txt");
        std::fs::write(&secret_a, "belongs to task-a only").unwrap();

        // The probe: a DIFFERENT task id's job directory must not contain, or
        // resolve to any path under, task-a's directory — a sibling trial
        // must have no way to see task-a's artifact through Crucible's own
        // directory layout.
        let job_dir_b = prepare_harbor_job_dir(&jobs_root, "task-b").expect("prepare task-b");
        assert_ne!(job_dir_a, job_dir_b, "distinct task ids get distinct slots");
        assert!(
            !job_dir_b.starts_with(&job_dir_a) && !job_dir_a.starts_with(&job_dir_b),
            "task-b's directory must not nest inside (or contain) task-a's: {job_dir_a:?} vs {job_dir_b:?}"
        );
        assert!(
            secret_a.exists(),
            "preparing a sibling task's job dir must not touch an unrelated task's artifacts"
        );
        assert!(
            std::fs::read_dir(&job_dir_b)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(true),
            "task-b's freshly prepared directory contains none of task-a's files"
        );

        let _ = std::fs::remove_dir_all(&jobs_root);
    }

    struct FakeModelClient {
        output: &'static str,
    }

    impl ModelClient for FakeModelClient {
        fn complete(&self, request: ModelRequest<'_>) -> anyhow::Result<ModelResponse> {
            Ok(ModelResponse {
                output: self.output.to_string(),
                response_id: Some(format!("fake:{}", request.model)),
                response_model: Some(request.model.to_string()),
                input_units: Some(7),
                output_units: Some(3),
                total_units: Some(10),
                cost_usd: Some(0.0),
            })
        }
    }

    /// A fake judge that replies with each queued output in call order — one
    /// per task, so different tasks (e.g. a real candidate vs. a canary) get
    /// distinct verdicts. Also records every `user_prompt` it was called
    /// with, in order, so tests can assert on exactly what was sent (e.g.
    /// that a reference exemplar was injected, or that a format-sensitivity
    /// probe reordered the prompt) without a separate mock.
    struct QueuedModelClient {
        outputs: std::cell::RefCell<std::collections::VecDeque<&'static str>>,
        prompts: std::cell::RefCell<Vec<String>>,
    }

    impl QueuedModelClient {
        fn new(outputs: Vec<&'static str>) -> Self {
            Self {
                outputs: std::cell::RefCell::new(outputs.into_iter().collect()),
                prompts: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn recorded_prompts(&self) -> Vec<String> {
            self.prompts.borrow().clone()
        }
    }

    impl ModelClient for QueuedModelClient {
        fn complete(&self, request: ModelRequest<'_>) -> anyhow::Result<ModelResponse> {
            self.prompts
                .borrow_mut()
                .push(request.user_prompt.to_string());
            let output = self
                .outputs
                .borrow_mut()
                .pop_front()
                .expect("QueuedModelClient ran out of queued outputs");
            Ok(ModelResponse {
                output: output.to_string(),
                response_id: Some(format!("fake:{}", request.model)),
                response_model: Some(request.model.to_string()),
                input_units: Some(7),
                output_units: Some(3),
                total_units: Some(10),
                cost_usd: Some(0.0),
            })
        }
    }

    fn agentic_judge_spec() -> EvalSpec {
        EvalSpec {
            schema_version: crucible_core::EVAL_SPEC_SCHEMA.to_string(),
            id: "agentic-judge-smoke".to_string(),
            context: None,
            task: "agentic-judge-smoke".to_string(),
            inputs: String::new(),
            outputs: String::new(),
            fixtures: Vec::new(),
            graders: crucible_core::GraderManifest {
                graders: vec![crucible_core::Grader {
                    id: "model-judge".to_string(),
                    kind: GraderKind::Agentic,
                }],
            },
            baselines: Vec::new(),
            aggregation: AggregationMethod::Proportion,
            uncertainty: crucible_core::UncertaintyRule::default(),
            decision: String::new(),
            min_effect_of_interest: None,
            runner: None,
        }
    }

    fn agentic_judge_config() -> AgenticJudgeConfig {
        AgenticJudgeConfig {
            provider: ModelProvider::OpenRouter,
            model: "test/judge".to_string(),
            judge_prompt: "Grade the candidate against the rubric.".to_string(),
            credential_env: "OPENROUTER_API_KEY".to_string(),
            temperature: Some(0),
            generator_model: None,
            harness: None,
            tool_allowlist: Vec::new(),
            format_sensitivity_check: false,
            previous_evidence_path: None,
        }
    }

    #[test]
    fn relative_spec_paths_resolve_from_spec_directory() {
        let spec_path = Path::new("/repo/evals/pr-review-key-recall-v0.json");
        let resolved = resolve_spec_path(spec_path, "../../daedalus/arenas/pr-review-v0");
        assert_eq!(
            resolved,
            Path::new("/repo/evals/../../daedalus/arenas/pr-review-v0")
        );
    }

    #[test]
    fn daedalus_paths_resolve_to_threshold_alias_when_that_checkout_exists() {
        let root =
            std::env::temp_dir().join(format!("crucible-daedalus-alias-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let spec_dir = root.join("crucible/evals");
        let threshold_arena = root.join("threshold/arenas/pr-review-v0");
        std::fs::create_dir_all(&spec_dir).expect("create spec dir");
        std::fs::create_dir_all(&threshold_arena).expect("create threshold arena dir");
        let spec_path = spec_dir.join("pr-review-key-recall-v0.json");

        let resolved =
            resolve_spec_path_with_alias(&spec_path, "../../daedalus/arenas/pr-review-v0");

        assert_eq!(
            resolved.path,
            std::fs::canonicalize(&threshold_arena).expect("canonical threshold arena")
        );
        assert_eq!(resolved.alias, Some("daedalus_to_threshold"));
    }

    #[test]
    fn prompt_model_override_updates_the_effective_config_only() {
        let config = PromptModelConfig {
            provider: ModelProvider::OpenRouter,
            model: "openrouter/auto".to_string(),
            system_prompt: "Answer exactly.".to_string(),
            credential_env: "OPENROUTER_API_KEY".to_string(),
            max_output_units: Some(8),
            temperature: Some(0),
            harness: None,
            tool_allowlist: Vec::new(),
        };

        let effective =
            prompt_config_with_overrides(&config, &RunOptions::with_prompt_model("test/model"));

        assert_eq!(config.model, "openrouter/auto");
        assert_eq!(effective.model, "test/model");
        assert_eq!(effective.system_prompt, config.system_prompt);
    }

    #[test]
    fn model_override_refuses_non_prompt_runners() {
        let temp =
            std::env::temp_dir().join(format!("crucible-model-override-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("key-recall.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");
        let spec = EvalSpec {
            schema_version: crucible_core::EVAL_SPEC_SCHEMA.to_string(),
            id: "key-recall".to_string(),
            context: None,
            task: "key-recall".to_string(),
            inputs: String::new(),
            outputs: String::new(),
            fixtures: Vec::new(),
            graders: crucible_core::GraderManifest {
                graders: vec![crucible_core::Grader {
                    id: "expected_key_match".to_string(),
                    kind: GraderKind::Deterministic,
                }],
            },
            baselines: Vec::new(),
            aggregation: AggregationMethod::Proportion,
            uncertainty: crucible_core::UncertaintyRule::default(),
            decision: String::new(),
            min_effect_of_interest: None,
            runner: None,
        };
        let runner = RunnerSpec {
            kind: RunnerKind::KeyRecall,
            corpus: CorpusSpec::DaedalusTrials {
                arena_dir: "arena".to_string(),
                trials_jsonl: "trials.jsonl".to_string(),
                candidate_id: "probe".to_string(),
                tasks: Vec::new(),
            },
        };

        let err = run_runner(
            &spec,
            &runner,
            &spec_path,
            &temp,
            &RunOptions::with_prompt_model("test/model"),
        )
        .expect_err("model override must not be ignored by key_recall");

        assert!(
            err.to_string().contains("prompt_benchmark"),
            "error names the constrained runner kind: {err}"
        );
    }

    #[test]
    fn null_chat_content_is_an_empty_answer_not_a_run_abort() {
        assert_eq!(
            chat_content_to_string(serde_json::Value::Null),
            Some(String::new()),
            "a model that returns no final content should fail the exact task, not abort the run"
        );
    }

    #[test]
    fn expected_key_scoring_uses_daedalus_span_contract_without_line_tolerance() {
        let expected = ExpectedKey {
            defects: vec![crucible_core::Defect {
                id: "d1".to_string(),
                file: "src/lib.rs".to_string(),
                line_start: 13,
                line_end: 14,
                category: "correctness".to_string(),
                severity: None,
                note: String::new(),
            }],
        };
        let just_outside = KeyFinding {
            file: "src/lib.rs".to_string(),
            line: 12,
            category: "correctness".to_string(),
            severity: String::new(),
            description: "near miss".to_string(),
            source_id: None,
        };
        let grade = crucible_core::score_against_expected_key(&[just_outside], &expected);
        assert_eq!(
            grade,
            crucible_core::SpanGrade {
                matched_ids: Vec::new(),
                missed_ids: vec!["d1".to_string()],
                false_positives: 1,
            }
        );
    }

    #[test]
    fn prompt_benchmark_runner_records_model_output_and_scores_rubric() {
        let temp = std::env::temp_dir().join(format!("crucible-prompt-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("prompt-smoke.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");
        let spec = EvalSpec {
            schema_version: crucible_core::EVAL_SPEC_SCHEMA.to_string(),
            id: "prompt-smoke".to_string(),
            context: None,
            task: "prompt-smoke".to_string(),
            inputs: String::new(),
            outputs: String::new(),
            fixtures: Vec::new(),
            graders: crucible_core::GraderManifest::default(),
            baselines: Vec::new(),
            aggregation: AggregationMethod::Proportion,
            uncertainty: crucible_core::UncertaintyRule::default(),
            decision: String::new(),
            min_effect_of_interest: None,
            runner: None,
        };
        let runner = RunnerSpec {
            kind: RunnerKind::PromptBenchmark,
            corpus: CorpusSpec::PromptBenchmark {
                config: PromptModelConfig {
                    provider: ModelProvider::OpenRouter,
                    model: "test/model".to_string(),
                    system_prompt: "Answer exactly.".to_string(),
                    credential_env: "OPENROUTER_API_KEY".to_string(),
                    max_output_units: Some(8),
                    temperature: Some(0),
                    harness: Some("claude-code".to_string()),
                    tool_allowlist: vec!["bash".to_string(), "web_search".to_string()],
                },
                tasks: vec![PromptBenchmarkTask {
                    task_id: "exact".to_string(),
                    class: Some("format_adherence".to_string()),
                    context_file: None,
                    prompt: "Reply with exactly: crucible-smoke".to_string(),
                    expectation: PromptExpectation::Exact {
                        value: "crucible-smoke".to_string(),
                    },
                    tracked: vec![crucible_core::TrackedCheck {
                        id: "mentions-tracked-marker".to_string(),
                        expectation: PromptExpectation::Contains {
                            value: "tracked-marker".to_string(),
                        },
                    }],
                }],
            },
        };

        let report = run_prompt_benchmark_with_client(
            &spec,
            &runner,
            &spec_path,
            &temp,
            match &runner.corpus {
                CorpusSpec::PromptBenchmark { config, .. } => config,
                _ => unreachable!(),
            },
            match &runner.corpus {
                CorpusSpec::PromptBenchmark { tasks, .. } => tasks,
                _ => unreachable!(),
            },
            &FakeModelClient {
                output: "crucible-smoke",
            },
        )
        .expect("prompt benchmark runs");

        assert_eq!(report.score.metric, "prompt_rubric_pass_rate");
        assert_eq!(report.score.successes, 1);
        assert_eq!(report.score.n, 1);
        let expected_score = wilson_score("prompt_rubric_pass_rate", 1, 1);
        assert_eq!(report.score.point, expected_score.point);
        assert_eq!(report.score.lower, expected_score.lower);
        assert_eq!(report.score.upper, expected_score.upper);
        assert_eq!(report.score.confidence, expected_score.confidence);
        let evidence = std::fs::read_to_string(temp.join("prompt-run.json"))
            .expect("prompt evidence is written");
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(
            evidence["schema_version"],
            "crucible.prompt_run_evidence.v1"
        );
        assert_eq!(evidence["tasks"][0]["output"], "crucible-smoke");
        assert_eq!(evidence["tasks"][0]["passed"], true);
        assert_eq!(
            evidence["tasks"][0]["tracked_results"],
            serde_json::json!([{ "id": "mentions-tracked-marker", "passed": false }]),
            "tracked outcomes are recorded separately from the gate verdict"
        );
        assert_eq!(evidence["tasks"][0]["class"], "format_adherence");
        assert_eq!(evidence["tasks"][0]["total_tokens"], 10);
        assert_eq!(evidence["max_output_units"], 8);
        assert_eq!(
            evidence["harness"], "claude-code",
            "the config's harness identity flows through to evidence: {evidence}"
        );
        assert_eq!(
            evidence["tool_allowlist"],
            serde_json::json!(["bash", "web_search"]),
            "the config's tool allowlist flows through to evidence: {evidence}"
        );
    }

    /// A model client that proves calls actually overlap in time: it counts
    /// in-flight calls, records the high-water mark, then sleeps briefly
    /// before answering. Every implementor field is an atomic, so this is
    /// `Sync` without any interior-mutability trickery — safe to share across
    /// worker threads the way the real `OpenRouterClient` is.
    struct ConcurrencyProbeClient {
        in_flight: std::sync::atomic::AtomicUsize,
        max_in_flight: std::sync::atomic::AtomicUsize,
        call_delay: Duration,
        output: &'static str,
    }

    impl ConcurrencyProbeClient {
        fn new(call_delay: Duration, output: &'static str) -> Self {
            Self {
                in_flight: std::sync::atomic::AtomicUsize::new(0),
                max_in_flight: std::sync::atomic::AtomicUsize::new(0),
                call_delay,
                output,
            }
        }

        fn max_in_flight(&self) -> usize {
            self.max_in_flight.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl ModelClient for ConcurrencyProbeClient {
        fn complete(&self, request: ModelRequest<'_>) -> anyhow::Result<ModelResponse> {
            use std::sync::atomic::Ordering;
            let now_in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight
                .fetch_max(now_in_flight, Ordering::SeqCst);
            thread::sleep(self.call_delay);
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(ModelResponse {
                output: self.output.to_string(),
                response_id: Some(format!("probe:{}", request.model)),
                response_model: Some(request.model.to_string()),
                input_units: Some(1),
                output_units: Some(1),
                total_units: Some(2),
                cost_usd: Some(0.0),
            })
        }
    }

    fn prompt_benchmark_task(task_id: &str) -> PromptBenchmarkTask {
        PromptBenchmarkTask {
            task_id: task_id.to_string(),
            class: None,
            context_file: None,
            prompt: format!("Reply with exactly: probe-ok ({task_id})"),
            expectation: PromptExpectation::Exact {
                value: "probe-ok".to_string(),
            },
            tracked: Vec::new(),
        }
    }

    #[test]
    fn prompt_benchmark_runs_task_model_calls_with_bounded_concurrency() {
        let temp = std::env::temp_dir().join(format!(
            "crucible-prompt-concurrency-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("prompt-concurrency.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = EvalSpec {
            schema_version: crucible_core::EVAL_SPEC_SCHEMA.to_string(),
            id: "prompt-concurrency".to_string(),
            context: None,
            task: "prompt-concurrency".to_string(),
            inputs: String::new(),
            outputs: String::new(),
            fixtures: Vec::new(),
            graders: crucible_core::GraderManifest::default(),
            baselines: Vec::new(),
            aggregation: AggregationMethod::Proportion,
            uncertainty: crucible_core::UncertaintyRule::default(),
            decision: String::new(),
            min_effect_of_interest: None,
            runner: None,
        };
        let task_ids = ["t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8"];
        let tasks: Vec<PromptBenchmarkTask> = task_ids
            .iter()
            .map(|id| prompt_benchmark_task(id))
            .collect();
        let config = PromptModelConfig {
            provider: ModelProvider::OpenRouter,
            model: "test/model".to_string(),
            system_prompt: "Answer exactly.".to_string(),
            credential_env: "OPENROUTER_API_KEY".to_string(),
            max_output_units: Some(8),
            temperature: Some(0),
            harness: None,
            tool_allowlist: Vec::new(),
        };
        let runner = RunnerSpec {
            kind: RunnerKind::PromptBenchmark,
            corpus: CorpusSpec::PromptBenchmark {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };

        // 8 tasks * 40ms each: fully sequential takes >= 320ms; bounded
        // concurrency (width > 1) should clear well under that.
        let client = ConcurrencyProbeClient::new(Duration::from_millis(40), "probe-ok");
        let started = Instant::now();
        let report = run_prompt_benchmark_with_client(
            &spec, &runner, &spec_path, &temp, &config, &tasks, &client,
        )
        .expect("prompt benchmark runs concurrently");
        let elapsed = started.elapsed();

        assert_eq!(
            report.score.successes, 8,
            "every task's fixed output passes"
        );
        assert_eq!(report.score.n, 8);
        assert!(
            client.max_in_flight() >= 2,
            "expected overlapping in-flight calls, saw a high-water mark of {}",
            client.max_in_flight()
        );
        assert!(
            elapsed < Duration::from_millis(250),
            "8 tasks at 40ms each ran in {elapsed:?}; sequential execution would take >= 320ms, \
             bounded concurrency should clear this well under 250ms"
        );

        let evidence = std::fs::read_to_string(temp.join("prompt-run.json"))
            .expect("prompt evidence is written");
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        let evidence_tasks = evidence["tasks"].as_array().unwrap();
        assert_eq!(evidence_tasks.len(), 8);
        // Task order in the persisted evidence matches input order regardless
        // of which worker thread finished first.
        let ids: Vec<&str> = evidence_tasks
            .iter()
            .map(|task| task["task_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, task_ids.to_vec());
    }

    #[test]
    fn prompt_benchmark_prepends_context_file_when_declared() {
        let temp =
            std::env::temp_dir().join(format!("crucible-prompt-context-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("prompt-context.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");
        std::fs::write(temp.join("doc.txt"), "The hidden code is XQ-17.").expect("write context");
        let task = PromptBenchmarkTask {
            task_id: "ctx".to_string(),
            class: Some("long_context_extraction".to_string()),
            context_file: Some("doc.txt".to_string()),
            prompt: "Return the hidden code.".to_string(),
            expectation: PromptExpectation::Exact {
                value: "XQ-17".to_string(),
            },
            tracked: Vec::new(),
        };

        let prompt = prompt_text_for_task(&spec_path, &task).expect("context prompt builds");

        assert!(prompt.contains("The hidden code is XQ-17."), "{prompt}");
        assert!(prompt.contains("Return the hidden code."), "{prompt}");
    }

    #[test]
    fn prompt_run_options_override_the_runner_bundle_fields() {
        let config = PromptModelConfig {
            provider: ModelProvider::OpenRouter,
            model: "test/model-a".to_string(),
            system_prompt: "Answer exactly.".to_string(),
            credential_env: "OPENROUTER_API_KEY".to_string(),
            max_output_units: Some(8),
            temperature: Some(0),
            harness: Some("claude-code".to_string()),
            tool_allowlist: vec!["bash".to_string()],
        };
        let options = RunOptions {
            prompt_model: Some("test/model-b".to_string()),
            prompt_system_prompt: Some("Use terse answers.".to_string()),
            prompt_max_output_units: Some(32),
            prompt_temperature: Some(1),
        };

        let effective = prompt_config_with_overrides(&config, &options);

        assert_eq!(effective.model, "test/model-b");
        assert_eq!(effective.system_prompt, "Use terse answers.");
        assert_eq!(effective.max_output_units, Some(32));
        assert_eq!(effective.temperature, Some(1));
        assert_eq!(
            effective.harness,
            Some("claude-code".to_string()),
            "harness identity is not one of the overridable fields; it survives untouched"
        );
        assert_eq!(
            effective.tool_allowlist,
            vec!["bash".to_string()],
            "tool allowlist is not one of the overridable fields; it survives untouched"
        );
    }

    #[test]
    fn prompt_expectation_case_insensitive_contains_ignores_case() {
        let expectation = PromptExpectation::CaseInsensitiveContains {
            value: "Crucible-Smoke".to_string(),
        };
        assert!(prompt_expectation_passes("this has crucible-smoke in it", &expectation).unwrap());
        assert!(prompt_expectation_passes("THIS HAS CRUCIBLE-SMOKE IN IT", &expectation).unwrap());
        assert!(!prompt_expectation_passes("no match here", &expectation).unwrap());
    }

    #[test]
    fn prompt_expectation_regex_matches_and_refuses_to_compile_garbage() {
        let expectation = PromptExpectation::Regex {
            pattern: r"^\d{3}-\d{4}$".to_string(),
        };
        assert!(prompt_expectation_passes("555-1234", &expectation).unwrap());
        assert!(!prompt_expectation_passes("not a phone number", &expectation).unwrap());

        let malformed = PromptExpectation::Regex {
            pattern: "(unclosed".to_string(),
        };
        let err = prompt_expectation_passes("anything", &malformed)
            .expect_err("an unclosed group must refuse to compile, not panic");
        assert!(
            err.to_string().contains("(unclosed"),
            "error names the offending pattern: {err}"
        );
    }

    #[test]
    fn prompt_expectation_strict_json_requires_parseable_exact_json() {
        let expectation = PromptExpectation::StrictJson {
            value: serde_json::json!({"answer":"CRU-42","ok":true}),
        };
        assert!(
            prompt_expectation_passes(r#"{"answer":"CRU-42","ok":true}"#, &expectation).unwrap()
        );
        assert!(
            !prompt_expectation_passes(r#"{"ok":true,"answer":"wrong"}"#, &expectation).unwrap()
        );
        assert!(!prompt_expectation_passes(
            r#"Here is JSON: {"answer":"CRU-42","ok":true}"#,
            &expectation,
        )
        .expect("strict_json treats prose-wrapped JSON as a miss"));
    }

    #[test]
    fn prompt_expectation_python_unit_test_executes_committed_test_source() {
        let expectation = PromptExpectation::PythonUnitTest {
            test_source:
                "from solution import normalize\nassert normalize(['b', 'a', 'b']) == ['a', 'b']\n"
                    .to_string(),
            timeout_ms: Some(1000),
        };
        assert!(prompt_expectation_passes(
            "def normalize(values):\n    return sorted(set(values))\n",
            &expectation
        )
        .expect("passing python unit test runs"));
        assert!(!prompt_expectation_passes(
            "def normalize(values):\n    return values\n",
            &expectation
        )
        .expect("failing python unit test runs"));
    }

    #[test]
    fn check_prompt_regexes_names_the_task_with_the_bad_pattern() {
        let tasks = vec![
            PromptBenchmarkTask {
                task_id: "fine".to_string(),
                class: None,
                context_file: None,
                prompt: "p".to_string(),
                expectation: PromptExpectation::Regex {
                    pattern: "ok".to_string(),
                },
                tracked: Vec::new(),
            },
            PromptBenchmarkTask {
                task_id: "broken".to_string(),
                class: None,
                context_file: None,
                prompt: "p".to_string(),
                expectation: PromptExpectation::Regex {
                    pattern: "[".to_string(),
                },
                tracked: Vec::new(),
            },
        ];
        let err = check_prompt_regexes(&tasks).expect_err("the second task's pattern is invalid");
        assert!(err.to_string().contains("broken"), "{err}");
    }

    #[test]
    fn run_prompt_benchmark_refuses_a_malformed_regex_before_any_model_call() {
        let temp =
            std::env::temp_dir().join(format!("crucible-prompt-bad-regex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("prompt-bad-regex.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");
        let spec = EvalSpec {
            schema_version: crucible_core::EVAL_SPEC_SCHEMA.to_string(),
            id: "prompt-bad-regex".to_string(),
            context: None,
            task: "prompt-bad-regex".to_string(),
            inputs: String::new(),
            outputs: String::new(),
            fixtures: Vec::new(),
            graders: crucible_core::GraderManifest {
                graders: vec![crucible_core::Grader {
                    id: "regex_rubric".to_string(),
                    kind: GraderKind::Deterministic,
                }],
            },
            baselines: Vec::new(),
            aggregation: AggregationMethod::Proportion,
            uncertainty: crucible_core::UncertaintyRule::default(),
            decision: String::new(),
            min_effect_of_interest: None,
            runner: None,
        };
        let runner = RunnerSpec {
            kind: RunnerKind::PromptBenchmark,
            corpus: CorpusSpec::PromptBenchmark {
                config: PromptModelConfig {
                    provider: ModelProvider::OpenRouter,
                    model: "test/model".to_string(),
                    system_prompt: "Answer exactly.".to_string(),
                    credential_env: "OPENROUTER_API_KEY".to_string(),
                    max_output_units: None,
                    temperature: None,
                    harness: None,
                    tool_allowlist: Vec::new(),
                },
                tasks: vec![PromptBenchmarkTask {
                    task_id: "broken".to_string(),
                    class: None,
                    context_file: None,
                    prompt: "irrelevant".to_string(),
                    expectation: PromptExpectation::Regex {
                        pattern: "(unclosed".to_string(),
                    },
                    tracked: Vec::new(),
                }],
            },
        };

        // No OPENROUTER_API_KEY is set in this test process, and the client
        // is never reached: run_prompt_benchmark checks every Regex pattern
        // before it even builds an OpenRouterClient, so if this refused for
        // the credential instead of the pattern, the assertion below would
        // catch it.
        let err = run_prompt_benchmark(&spec, &runner, &spec_path, &temp, &RunOptions::default())
            .expect_err("a malformed regex must refuse before any model call");
        let full_chain = format!("{err:#}");
        assert!(
            full_chain.contains("(unclosed") && full_chain.contains("broken"),
            "error chain names the pattern and the task, not a credential complaint: {full_chain}"
        );
    }

    #[test]
    fn agentic_judge_scores_real_tasks_and_confirms_the_canary() {
        let temp = std::env::temp_dir().join(format!("crucible-judge-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-smoke.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config();
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct, well-reasoned answer.".to_string(),
                rubric: "The answer must be correct and well-reasoned.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "canary".to_string(),
                candidate: "This answer is nonsense and ignores the question.".to_string(),
                rubric: "The answer must be correct and well-reasoned.".to_string(),
                expected_pass: Some(false),
                refuse_on_mismatch: true,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };

        // The real task's candidate earns VERDICT: PASS; the canary's bad
        // candidate correctly earns VERDICT: FAIL — the judge is not gaming.
        let client = QueuedModelClient::new(vec![
            "The answer is correct.\nVERDICT: PASS",
            "The answer does not address the rubric.\nVERDICT: FAIL",
        ]);

        let report = run_agentic_judge_with_client(
            &spec, &runner, &spec_path, &temp, &config, &tasks, &client,
        )
        .expect("agentic judge runs");

        assert_eq!(report.score.metric, "judge_pass_rate");
        assert_eq!(report.score.successes, 1);
        assert_eq!(report.score.n, 1, "the canary is excluded from the score");
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("matched its expected verdict")),
            "notes record the canary's agreement: {:?}",
            report.notes
        );

        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json"))
            .expect("agentic judge evidence is written");
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(
            evidence["schema_version"],
            "crucible.agentic_judge_evidence.v1"
        );
        assert_eq!(evidence["tasks"].as_array().unwrap().len(), 2);
        assert_eq!(evidence["tasks"][0]["task_id"], "real-1");
        assert_eq!(evidence["tasks"][0]["passed"], true);
        assert_eq!(evidence["tasks"][1]["task_id"], "canary");
        assert_eq!(evidence["tasks"][1]["passed"], false);
        assert_eq!(evidence["tasks"][1]["expected_pass"], false);

        assert_eq!(
            evidence["calibration"]["schema_version"],
            "crucible.calibration_record.v1"
        );
        assert_eq!(evidence["calibration"]["judge_id"], "test/judge");
        assert_eq!(evidence["calibration"]["n"], 1);
        assert_eq!(evidence["calibration"]["agreement"], 1.0);
        assert_eq!(evidence["calibration"]["unlocked"], true);
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("Calibration UNLOCKED")),
            "notes record the calibration unlock state: {:?}",
            report.notes
        );
    }

    #[test]
    fn agentic_judge_calibration_locks_below_the_agreement_threshold() {
        let temp = std::env::temp_dir().join(format!(
            "crucible-judge-miscalibrated-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-miscalibrated.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config();
        // One real task plus three non-refusing calibration probes; the judge
        // disagrees with two of three (agreement 1/3 ≈ 0.33 < 0.8 threshold).
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "calib-agree".to_string(),
                candidate: "Agrees with the judge.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(true),
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "calib-disagree-1".to_string(),
                candidate: "Disagrees with the judge.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(true),
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "calib-disagree-2".to_string(),
                candidate: "Also disagrees with the judge.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(false),
                refuse_on_mismatch: false,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec![
            "real task passes\nVERDICT: PASS",
            "agrees\nVERDICT: PASS",
            "disagrees with expected true\nVERDICT: FAIL",
            "disagrees with expected false\nVERDICT: PASS",
        ]);

        let report = run_agentic_judge_with_client(
            &spec, &runner, &spec_path, &temp, &config, &tasks, &client,
        )
        .expect("agentic judge runs; non-refusing calibration mismatches do not abort");

        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json"))
            .expect("agentic judge evidence is written");
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(evidence["calibration"]["n"], 3);
        let agreement = evidence["calibration"]["agreement"].as_f64().unwrap();
        assert!(
            (agreement - (1.0 / 3.0)).abs() < 1e-9,
            "agreement is 1/3: {agreement}"
        );
        assert_eq!(
            evidence["calibration"]["unlocked"], false,
            "1/3 agreement does not clear the 0.8 threshold"
        );
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("Calibration LOCKED")),
            "notes record the diagnostic (locked) calibration state: {:?}",
            report.notes
        );

        // Confusion here is TP=1 (calib-agree), FN=1 (calib-disagree-1:
        // expected true, judge said false), FP=1 (calib-disagree-2: expected
        // false, judge said true), TN=0 — named FP/FN rate fields, not just
        // the aggregate agreement/κ (report §6 item 7 / §11).
        let fp_rate = evidence["calibration"]["false_positive_rate"]
            .as_f64()
            .unwrap();
        let fn_rate = evidence["calibration"]["false_negative_rate"]
            .as_f64()
            .unwrap();
        assert!(
            (fp_rate - 1.0).abs() < 1e-9,
            "FP rate is 1/(1+0): {fp_rate}"
        );
        assert!(
            (fn_rate - 0.5).abs() < 1e-9,
            "FN rate is 1/(1+1): {fn_rate}"
        );

        // Calibration unlock state is queryable across runs by a stable
        // licence key (model + judge prompt + calibration rubric set + task
        // family, backlog 970).
        let licence_key = evidence["calibration"]["licence_key"].as_str().unwrap();
        assert!(!licence_key.is_empty());
        assert!(licence_key.starts_with("judge-licence:v2:test/judge:"));
        assert!(
            licence_key.ends_with(":agentic-judge-smoke"),
            "the task family is folded into the licence key: {licence_key}"
        );

        // Fail-class precision/recall (backlog 970): TN=0, FN=1, FP=1 — the
        // judge never correctly called fail, so both are 0.0.
        assert_eq!(
            evidence["calibration"]["fail_class_precision"]
                .as_f64()
                .unwrap(),
            0.0
        );
        assert_eq!(
            evidence["calibration"]["fail_class_recall"]
                .as_f64()
                .unwrap(),
            0.0
        );
        assert_eq!(
            evidence["calibration"]["task_family"].as_str().unwrap(),
            "agentic-judge-smoke"
        );
        // No previous_evidence_path was configured, so no drift check ran.
        assert!(evidence["calibration"]["drift_flip_rate"].is_null());
    }

    #[test]
    fn agentic_judge_refuses_the_run_when_the_canary_is_rubber_stamped() {
        let temp =
            std::env::temp_dir().join(format!("crucible-judge-gaming-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-gaming.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config();
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct, well-reasoned answer.".to_string(),
                rubric: "The answer must be correct and well-reasoned.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "canary".to_string(),
                candidate: "This answer is nonsense and ignores the question.".to_string(),
                rubric: "The answer must be correct and well-reasoned.".to_string(),
                expected_pass: Some(false),
                refuse_on_mismatch: true,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };

        // A rubber-stamping judge: it passes the canary's obviously-bad
        // candidate too. The guard must refuse the whole run.
        let client = QueuedModelClient::new(vec![
            "Looks fine.\nVERDICT: PASS",
            "Looks fine.\nVERDICT: PASS",
        ]);

        let err = run_agentic_judge_with_client(
            &spec, &runner, &spec_path, &temp, &config, &tasks, &client,
        )
        .expect_err("a rubber-stamped canary must refuse the run");
        assert!(
            err.to_string().contains("judge-gaming guard tripped"),
            "error names the judge-gaming guard: {err}"
        );
        assert!(
            !temp.join("agentic-judge-run.json").exists(),
            "a refused run must not persist evidence as if it were trusted"
        );
    }

    #[test]
    fn agentic_judge_records_self_evaluation_bias_risk_when_judge_and_generator_share_a_family() {
        let temp =
            std::env::temp_dir().join(format!("crucible-judge-bias-risk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-bias-risk.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let mut config = agentic_judge_config();
        config.model = "openai/gpt-4o".to_string();
        config.generator_model = Some("openai/gpt-4o-mini".to_string());
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "calib-1".to_string(),
                candidate: "Agrees.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(true),
                refuse_on_mismatch: false,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec!["real\nVERDICT: PASS", "agrees\nVERDICT: PASS"]);

        let report = run_agentic_judge_with_client(
            &spec, &runner, &spec_path, &temp, &config, &tasks, &client,
        )
        .expect("agentic judge runs");

        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("Self-evaluation bias risk")),
            "notes surface the same-family risk rather than silently allowing it: {:?}",
            report.notes
        );

        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json")).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(evidence["calibration"]["self_evaluation_bias_risk"], true);
        assert_eq!(
            evidence["calibration"]["generator_id"],
            "openai/gpt-4o-mini"
        );
    }

    #[test]
    fn agentic_judge_records_no_bias_risk_for_a_different_generator_family() {
        let temp = std::env::temp_dir().join(format!(
            "crucible-judge-no-bias-risk-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-no-bias-risk.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let mut config = agentic_judge_config();
        config.model = "openai/gpt-4o".to_string();
        config.generator_model = Some("anthropic/claude-opus-4".to_string());
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "calib-1".to_string(),
                candidate: "Agrees.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(true),
                refuse_on_mismatch: false,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec!["real\nVERDICT: PASS", "agrees\nVERDICT: PASS"]);

        let report = run_agentic_judge_with_client(
            &spec, &runner, &spec_path, &temp, &config, &tasks, &client,
        )
        .expect("agentic judge runs");
        assert!(
            !report
                .notes
                .iter()
                .any(|note| note.contains("Self-evaluation bias risk")),
            "a different generator family must not raise a bias-risk note: {:?}",
            report.notes
        );

        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json")).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(evidence["calibration"]["self_evaluation_bias_risk"], false);
    }

    #[test]
    fn agentic_judge_unknown_scored_verdict_is_excluded_not_coerced() {
        let temp = std::env::temp_dir().join(format!(
            "crucible-judge-unknown-scored-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-unknown-scored.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config();
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "real-2-ambiguous".to_string(),
                candidate: "An underspecified answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec![
            "real\nVERDICT: PASS",
            "not enough information in the rubric to decide\nVERDICT: UNKNOWN",
        ]);

        let report = run_agentic_judge_with_client(
            &spec, &runner, &spec_path, &temp, &config, &tasks, &client,
        )
        .expect("an UNKNOWN scored verdict does not abort the run");

        assert_eq!(
            report.score.n, 1,
            "the UNKNOWN task is excluded, not counted as a fail"
        );
        assert_eq!(report.score.successes, 1);
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("excluded from the score's denominator")),
            "notes explain the exclusion: {:?}",
            report.notes
        );

        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json")).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(evidence["tasks"][1]["verdict"], "unknown");
        assert_eq!(
            evidence["tasks"][1]["passed"], false,
            "the legacy bool mirror never reads UNKNOWN as a pass"
        );
        assert_eq!(evidence["judge_stats"]["unknown_verdict_count"], 1);
        assert!(evidence["judge_stats"]["failure_rate"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn agentic_judge_unknown_canary_verdict_does_not_trip_the_gaming_guard() {
        let temp = std::env::temp_dir().join(format!(
            "crucible-judge-unknown-canary-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-unknown-canary.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config();
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "canary".to_string(),
                candidate: "Nonsense.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(false),
                refuse_on_mismatch: true,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        // An honest judge that admits it cannot tell on the canary. This is
        // not rubber-stamping (it never says PASS), so the guard must not
        // trip — but the disagreement/agreement measurement must exclude it.
        let client = QueuedModelClient::new(vec![
            "real\nVERDICT: PASS",
            "cannot determine from the given rubric\nVERDICT: UNKNOWN",
        ]);

        let report = run_agentic_judge_with_client(
            &spec, &runner, &spec_path, &temp, &config, &tasks, &client,
        )
        .expect("an UNKNOWN verdict on a canary must not trip the judge-gaming guard");

        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("returned UNKNOWN — diagnostic")),
            "notes record the excluded calibration probe: {:?}",
            report.notes
        );

        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json")).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert!(
            evidence["calibration"].is_null(),
            "no decisive calibration verdicts were measured: {evidence}"
        );
        assert_eq!(evidence["judge_stats"]["unknown_verdict_count"], 1);
    }

    #[test]
    fn agentic_judge_evidence_records_judge_run_stats() {
        let temp =
            std::env::temp_dir().join(format!("crucible-judge-run-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-run-stats.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config();
        let tasks = vec![AgenticJudgeTask {
            task_id: "real-1".to_string(),
            candidate: "A correct answer.".to_string(),
            rubric: "Must be correct.".to_string(),
            expected_pass: None,
            refuse_on_mismatch: false,
            reference: None,
        }];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec!["real\nVERDICT: PASS"]);

        run_agentic_judge_with_client(&spec, &runner, &spec_path, &temp, &config, &tasks, &client)
            .expect("agentic judge runs");

        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json")).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(evidence["judge_stats"]["call_count"], 1);
        assert_eq!(evidence["judge_stats"]["unknown_verdict_count"], 0);
        assert_eq!(evidence["judge_stats"]["failure_rate"], 0.0);
        assert!(evidence["judge_stats"]["total_cost_usd"].is_number());
        assert!(evidence["judge_stats"]["total_latency_ms"].is_u64());
    }

    #[test]
    fn agentic_judge_injects_reference_exemplar_labeled_not_as_the_candidate() {
        let temp =
            std::env::temp_dir().join(format!("crucible-judge-reference-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-reference.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config();
        let tasks = vec![AgenticJudgeTask {
            task_id: "real-1".to_string(),
            candidate: "A partially correct answer.".to_string(),
            rubric: "Must fully answer the question.".to_string(),
            expected_pass: None,
            refuse_on_mismatch: false,
            reference: Some("The known-perfect answer text.".to_string()),
        }];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec!["real\nVERDICT: PASS"]);

        run_agentic_judge_with_client(&spec, &runner, &spec_path, &temp, &config, &tasks, &client)
            .expect("agentic judge runs");

        let prompts = client.recorded_prompts();
        assert_eq!(prompts.len(), 1);
        assert!(
            prompts[0].contains("The known-perfect answer text."),
            "the reference exemplar is injected into the judge's prompt: {:?}",
            prompts[0]
        );
        assert!(
            prompts[0].contains("known-perfect exemplar"),
            "the reference is labeled as a known-perfect exemplar, not presented as the candidate: {:?}",
            prompts[0]
        );
        assert!(
            prompts[0].contains("A partially correct answer."),
            "the actual candidate is still present and distinguishable from the reference: {:?}",
            prompts[0]
        );
    }

    #[test]
    fn agentic_judge_omits_the_reference_block_when_no_reference_is_declared() {
        let temp = std::env::temp_dir().join(format!(
            "crucible-judge-no-reference-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-no-reference.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config();
        let tasks = vec![AgenticJudgeTask {
            task_id: "real-1".to_string(),
            candidate: "An answer.".to_string(),
            rubric: "Must be correct.".to_string(),
            expected_pass: None,
            refuse_on_mismatch: false,
            reference: None,
        }];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec!["real\nVERDICT: PASS"]);

        run_agentic_judge_with_client(&spec, &runner, &spec_path, &temp, &config, &tasks, &client)
            .expect("agentic judge runs");

        let prompts = client.recorded_prompts();
        assert!(
            !prompts[0].contains("known-perfect exemplar"),
            "no reference block appears when the task declares no reference: {:?}",
            prompts[0]
        );
    }

    #[test]
    fn format_sensitivity_check_records_a_zero_flip_rate_when_the_judge_is_stable() {
        let temp = std::env::temp_dir().join(format!(
            "crucible-judge-format-sensitivity-stable-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-format-sensitivity-stable.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let mut config = agentic_judge_config();
        config.format_sensitivity_check = true;
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "calib-1".to_string(),
                candidate: "Agrees.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(true),
                refuse_on_mismatch: false,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        // Two ordinary calls, then one format-sensitivity re-probe of the one
        // decisive calibration item ("calib-1") — same verdict both times, so
        // the flip rate is 0.0, not None (checked, and stable).
        let client = QueuedModelClient::new(vec![
            "real\nVERDICT: PASS",
            "agrees\nVERDICT: PASS",
            "still agrees under reordering\nVERDICT: PASS",
        ]);

        run_agentic_judge_with_client(&spec, &runner, &spec_path, &temp, &config, &tasks, &client)
            .expect("agentic judge runs");

        let prompts = client.recorded_prompts();
        assert_eq!(
            prompts.len(),
            3,
            "the probe call is a real extra model call"
        );
        assert!(
            prompts[2].starts_with("Candidate output:"),
            "the format-sensitivity probe reorders the prompt (candidate section first): {:?}",
            prompts[2]
        );

        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json")).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(evidence["calibration"]["format_sensitivity_flip_rate"], 0.0);
        assert_eq!(evidence["calibration"]["format_sensitivity_n"], 1);
    }

    #[test]
    fn format_sensitivity_check_records_a_nonzero_flip_rate_when_the_judge_is_fragile() {
        let temp = std::env::temp_dir().join(format!(
            "crucible-judge-format-sensitivity-fragile-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-format-sensitivity-fragile.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let mut config = agentic_judge_config();
        config.format_sensitivity_check = true;
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "calib-1".to_string(),
                candidate: "Agrees.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(true),
                refuse_on_mismatch: false,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        // Original verdict PASS; the cosmetically reordered re-probe flips to
        // FAIL — a judge whose verdict is sensitive to a purely cosmetic
        // perturbation, which the flip rate must surface.
        let client = QueuedModelClient::new(vec![
            "real\nVERDICT: PASS",
            "agrees\nVERDICT: PASS",
            "flips under reordering\nVERDICT: FAIL",
        ]);

        run_agentic_judge_with_client(&spec, &runner, &spec_path, &temp, &config, &tasks, &client)
            .expect("agentic judge runs; format-sensitivity flips do not abort the run");

        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json")).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(evidence["calibration"]["format_sensitivity_flip_rate"], 1.0);
        assert_eq!(evidence["calibration"]["format_sensitivity_n"], 1);
    }

    #[test]
    fn format_sensitivity_check_is_opt_in_and_absent_by_default() {
        let temp = std::env::temp_dir().join(format!(
            "crucible-judge-format-sensitivity-off-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-format-sensitivity-off.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config(); // format_sensitivity_check: false
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "calib-1".to_string(),
                candidate: "Agrees.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(true),
                refuse_on_mismatch: false,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec!["real\nVERDICT: PASS", "agrees\nVERDICT: PASS"]);

        run_agentic_judge_with_client(&spec, &runner, &spec_path, &temp, &config, &tasks, &client)
            .expect("agentic judge runs");

        assert_eq!(
            client.recorded_prompts().len(),
            2,
            "no extra probe call is made when format_sensitivity_check is false"
        );
        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json")).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert!(
            evidence["calibration"]["format_sensitivity_flip_rate"].is_null(),
            "an unrun check reports None, not a fabricated rate: {evidence}"
        );
        assert_eq!(evidence["calibration"]["format_sensitivity_n"], 0);
    }

    #[test]
    fn drift_check_records_the_flip_rate_against_a_prior_run_when_configured() {
        // Baseline run: the judge agrees with expected_pass on the one
        // calibration task.
        let baseline_out = std::env::temp_dir().join(format!(
            "crucible-judge-drift-baseline-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&baseline_out);
        std::fs::create_dir_all(&baseline_out).expect("create baseline dir");
        let baseline_spec_path = baseline_out.join("agentic-judge-drift.json");
        std::fs::write(&baseline_spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config(); // previous_evidence_path: None
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "calib-1".to_string(),
                candidate: "Agrees.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(true),
                refuse_on_mismatch: false,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let baseline_client =
            QueuedModelClient::new(vec!["real\nVERDICT: PASS", "agrees\nVERDICT: PASS"]);
        run_agentic_judge_with_client(
            &spec,
            &runner,
            &baseline_spec_path,
            &baseline_out,
            &config,
            &tasks,
            &baseline_client,
        )
        .expect("baseline run succeeds");
        let baseline_evidence_path = baseline_out.join("agentic-judge-run.json");
        assert!(
            evidence_task_verdict(&baseline_evidence_path, "calib-1") == Some(true),
            "baseline judge verdict on calib-1 is PASS"
        );

        // Second run over the *same* probe set (same task id), configured to
        // compare against the baseline evidence — this time the judge's
        // verdict flips to FAIL.
        let current_out = std::env::temp_dir().join(format!(
            "crucible-judge-drift-current-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&current_out);
        std::fs::create_dir_all(&current_out).expect("create current dir");
        let current_spec_path = current_out.join("agentic-judge-drift.json");
        std::fs::write(&current_spec_path, "{}").expect("write placeholder spec path");

        let mut drifted_config = config.clone();
        drifted_config.previous_evidence_path = Some(baseline_evidence_path.clone());
        let drifted_runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: drifted_config.clone(),
                tasks: tasks.clone(),
            },
        };
        let current_client =
            QueuedModelClient::new(vec!["real\nVERDICT: PASS", "disagrees now\nVERDICT: FAIL"]);
        run_agentic_judge_with_client(
            &spec,
            &drifted_runner,
            &current_spec_path,
            &current_out,
            &drifted_config,
            &tasks,
            &current_client,
        )
        .expect("current run succeeds; a locked calibration does not abort");

        let evidence = std::fs::read_to_string(current_out.join("agentic-judge-run.json")).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(
            evidence["calibration"]["drift_flip_rate"], 1.0,
            "the one shared probe task flipped from PASS to FAIL: {evidence}"
        );
        assert_eq!(evidence["calibration"]["drift_probe_n"], 1);
        assert!(
            evidence["calibration"]["drift_checked_at"]
                .as_i64()
                .is_some(),
            "a drift check that ran stamps a timestamp: {evidence}"
        );
    }

    #[test]
    fn drift_check_is_none_by_default_when_no_previous_evidence_is_configured() {
        let temp =
            std::env::temp_dir().join(format!("crucible-judge-drift-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-drift-off.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config(); // previous_evidence_path: None
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "calib-1".to_string(),
                candidate: "Agrees.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: Some(true),
                refuse_on_mismatch: false,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec!["real\nVERDICT: PASS", "agrees\nVERDICT: PASS"]);
        run_agentic_judge_with_client(&spec, &runner, &spec_path, &temp, &config, &tasks, &client)
            .expect("agentic judge runs");

        let evidence = std::fs::read_to_string(temp.join("agentic-judge-run.json")).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert!(
            evidence["calibration"]["drift_flip_rate"].is_null(),
            "an unrun drift check reports None, not a fabricated rate: {evidence}"
        );
        assert_eq!(evidence["calibration"]["drift_probe_n"], 0);
        assert!(evidence["calibration"]["drift_checked_at"].is_null());
    }

    /// Test helper: read a written evidence file's judge verdict (PASS=true,
    /// FAIL=false) for one task id.
    fn evidence_task_verdict(evidence_path: &Path, task_id: &str) -> Option<bool> {
        let raw = std::fs::read_to_string(evidence_path).unwrap();
        let evidence: serde_json::Value = serde_json::from_str(&raw).unwrap();
        evidence["tasks"].as_array()?.iter().find_map(|task| {
            if task["task_id"].as_str() != Some(task_id) {
                return None;
            }
            match task["verdict"].as_str() {
                Some("pass") => Some(true),
                Some("fail") => Some(false),
                _ => None,
            }
        })
    }

    #[test]
    fn agentic_judge_unknown_verdict_produces_an_inspectable_trace() {
        let temp = std::env::temp_dir().join(format!(
            "crucible-judge-trace-unknown-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-trace-unknown.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config();
        let tasks = vec![
            AgenticJudgeTask {
                task_id: "real-1".to_string(),
                candidate: "A correct answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
            AgenticJudgeTask {
                task_id: "real-2-ambiguous".to_string(),
                candidate: "An underspecified answer.".to_string(),
                rubric: "Must be correct.".to_string(),
                expected_pass: None,
                refuse_on_mismatch: false,
                reference: None,
            },
        ];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec![
            "real\nVERDICT: PASS",
            "not enough information in the rubric to decide\nVERDICT: UNKNOWN",
        ]);

        let report = run_agentic_judge_with_client(
            &spec, &runner, &spec_path, &temp, &config, &tasks, &client,
        )
        .expect("an UNKNOWN scored verdict does not abort the run");

        let trace_path = report
            .artifacts
            .iter()
            .find(|path| path.ends_with("agentic-judge-trace.json"))
            .expect("run artifacts include a trace pointer");
        let trace_json = std::fs::read_to_string(trace_path).expect("read trace artifact");
        let trace: crucible_core::Trace =
            serde_json::from_str(&trace_json).expect("trace parses as crucible_core::Trace");

        assert_eq!(trace.schema_version, crucible_core::TRACE_SCHEMA);
        assert!(
            !trace.steps.is_empty(),
            "a real run leaves a nonempty trace"
        );
        for (index, step) in trace.steps.iter().enumerate() {
            assert_eq!(
                step.sequence, index as u64,
                "steps are ordered by an ascending sequence, gapless from 0"
            );
            assert!(!step.kind.is_empty(), "every step names a kind");
        }

        // The task that got an honest UNKNOWN must be inspectable: a
        // verdict_parsed step labeled for that task, outcome "unknown", and
        // enough detail (the judge's raw output) to see *why* without
        // re-running the judge call.
        let unknown_step = trace
            .steps
            .iter()
            .find(|step| {
                step.kind == "verdict_parsed"
                    && step.label == "real-2-ambiguous"
                    && step.outcome.as_deref() == Some("unknown")
            })
            .expect("an inspectable verdict_parsed step for the UNKNOWN task");
        let detail_text = unknown_step.detail.to_string();
        assert!(
            detail_text.contains("not enough information"),
            "trace detail carries the judge's raw output for a no-re-run diagnosis: {detail_text}"
        );

        // The failing/ambiguous step is exactly what `failure_steps` surfaces.
        let failures: Vec<&str> = trace
            .failure_steps()
            .map(|step| step.label.as_str())
            .collect();
        assert_eq!(
            failures,
            vec!["real-2-ambiguous"],
            "failure_steps surfaces only the UNKNOWN task's step: {failures:?}"
        );

        // A judge_call step precedes its verdict_parsed step for the same task.
        let call_index = trace
            .steps
            .iter()
            .position(|step| step.kind == "judge_call" && step.label == "real-2-ambiguous")
            .expect("a judge_call step for the UNKNOWN task");
        let verdict_index = trace
            .steps
            .iter()
            .position(|step| step.kind == "verdict_parsed" && step.label == "real-2-ambiguous")
            .expect("a verdict_parsed step for the UNKNOWN task");
        assert!(
            call_index < verdict_index,
            "the call precedes its parsed verdict in trace order"
        );
    }

    #[test]
    fn agentic_judge_passing_run_trace_is_structurally_sound() {
        let temp =
            std::env::temp_dir().join(format!("crucible-judge-trace-pass-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-trace-pass.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let spec = agentic_judge_spec();
        let config = agentic_judge_config();
        let tasks = vec![AgenticJudgeTask {
            task_id: "real-1".to_string(),
            candidate: "A correct answer.".to_string(),
            rubric: "Must be correct.".to_string(),
            expected_pass: None,
            refuse_on_mismatch: false,
            reference: None,
        }];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: config.clone(),
                tasks: tasks.clone(),
            },
        };
        let client = QueuedModelClient::new(vec!["real\nVERDICT: PASS"]);

        let report = run_agentic_judge_with_client(
            &spec, &runner, &spec_path, &temp, &config, &tasks, &client,
        )
        .expect("agentic judge runs");

        let trace_path = report
            .artifacts
            .iter()
            .find(|path| path.ends_with("agentic-judge-trace.json"))
            .expect("run artifacts include a trace pointer");
        let trace: crucible_core::Trace = serde_json::from_str(
            &std::fs::read_to_string(trace_path).expect("read trace artifact"),
        )
        .expect("trace parses");

        assert!(!trace.steps.is_empty());
        assert_eq!(
            trace.failure_steps().count(),
            0,
            "a clean pass has no failure steps"
        );
        assert!(trace
            .steps
            .iter()
            .any(|step| step.kind == "verdict_parsed" && step.outcome.as_deref() == Some("pass")));
    }

    #[test]
    fn agentic_judge_requires_a_declared_agentic_grader() {
        let temp =
            std::env::temp_dir().join(format!("crucible-judge-no-grader-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("create temp dir");
        let spec_path = temp.join("agentic-judge-no-grader.json");
        std::fs::write(&spec_path, "{}").expect("write placeholder spec path");

        let mut spec = agentic_judge_spec();
        spec.graders = crucible_core::GraderManifest::default();
        let config = agentic_judge_config();
        let tasks = vec![AgenticJudgeTask {
            task_id: "real-1".to_string(),
            candidate: "A correct answer.".to_string(),
            rubric: "Must be correct.".to_string(),
            expected_pass: None,
            refuse_on_mismatch: false,
            reference: None,
        }];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge { config, tasks },
        };

        let err = run_runner(&spec, &runner, &spec_path, &temp, &RunOptions::default())
            .expect_err("a spec without a declared Agentic grader must refuse to run");
        assert!(
            err.to_string().contains("requires an Agentic grader"),
            "error names the missing grader declaration: {err}"
        );
    }

    #[test]
    fn parse_judge_verdict_reads_the_final_line_reasoning_first() {
        assert_eq!(
            parse_judge_verdict("The candidate is correct.\nVERDICT: PASS").unwrap(),
            JudgeVerdict::Pass
        );
        assert_eq!(
            parse_judge_verdict("The candidate misses the rubric.\nVERDICT: FAIL").unwrap(),
            JudgeVerdict::Fail
        );
        assert!(parse_judge_verdict("no verdict here").is_err());
        assert!(parse_judge_verdict("VERDICT: PASS and also VERDICT: FAIL").is_err());
    }

    #[test]
    fn parse_judge_verdict_rejects_the_old_verdict_first_format() {
        // Pre-2026-07-06 format: the tag came first, reasoning after. The
        // reasoning-first protocol requires the tag as the FINAL line, so a
        // response shaped like the old protocol must be rejected, not
        // silently accepted from wherever the tag happens to sit.
        let err = parse_judge_verdict("VERDICT: PASS\nThe answer is correct.").unwrap_err();
        assert!(
            err.to_string().contains("final line"),
            "error names the final-line requirement: {err}"
        );
    }

    #[test]
    fn parse_judge_verdict_takes_the_last_tag_ignoring_earlier_mentions() {
        // Reasoning that mentions "VERDICT: FAIL" in passing (e.g. weighing
        // it and then rejecting it) must not create ambiguity — only the
        // tag on the true final line counts.
        let output = "A stricter reading might suggest VERDICT: FAIL, but on balance the candidate meets the rubric.\nVERDICT: PASS";
        assert_eq!(parse_judge_verdict(output).unwrap(), JudgeVerdict::Pass);
    }

    #[test]
    fn parse_judge_verdict_accepts_an_explicit_unknown() {
        assert_eq!(
            parse_judge_verdict("not enough information to decide\nVERDICT: UNKNOWN").unwrap(),
            JudgeVerdict::Unknown
        );
        assert!(
            parse_judge_verdict("VERDICT: PASS and also VERDICT: UNKNOWN").is_err(),
            "UNKNOWN alongside another tag on the final line is still ambiguous, not a silent fallback"
        );
        assert!(parse_judge_verdict("VERDICT: FAIL and also VERDICT: UNKNOWN").is_err());
    }

    #[test]
    fn parse_judge_verdict_refuses_an_empty_response() {
        assert!(parse_judge_verdict("").is_err());
        assert!(parse_judge_verdict("   \n  \n").is_err());
    }

    fn task_result(task_id: &str, missed: u64, false_positives: u64) -> TaskResult {
        TaskResult {
            task_id: task_id.to_string(),
            run_id: format!("{task_id}-run"),
            trial: Some(1),
            candidate_id: "incumbent".to_string(),
            candidate_kind: None,
            arena_id: None,
            arena_version: None,
            key: String::new(),
            findings: 0,
            dropped_invalid: 0,
            matched: 0,
            matched_ids: Vec::new(),
            missed,
            missed_ids: Vec::new(),
            disputed: false_positives,
            false_positives,
            recoverable_misses: 0,
            expected_defects: 0,
            daedalus_reward: None,
            daedalus_recall: None,
            daedalus_false_positives: None,
            error: None,
            scorer_error: None,
            artifacts: None,
            artifact: None,
            receipt_bundle: None,
            receipt_harness: None,
            receipt_model: None,
            receipt_validation: None,
            receipt_trusted_for_posting: None,
        }
    }

    #[test]
    fn pass_k_counts_tasks_that_fully_matched_on_every_trial() {
        // Two tasks, k=5 trials each: task "a" fully matches all 5 trials;
        // task "b" has one trial with a missed defect. pass^5 = 1/2.
        let mut results = Vec::new();
        for trial in 0..5 {
            results.push(task_result("a", 0, 0));
            let miss = if trial == 2 { 1 } else { 0 };
            results.push(task_result("b", miss, 0));
        }
        let pass_k = compute_pass_k(&results).expect("uniform k=5 reports pass^k");
        assert_eq!(pass_k.k, 5);
        assert_eq!(pass_k.score.successes, 1);
        assert_eq!(pass_k.score.n, 2);
        assert_eq!(pass_k.score.metric, "pass_k_task_consistency");
    }

    #[test]
    fn pass_k_counts_a_false_positive_trial_as_not_passed() {
        let mut results = Vec::new();
        for _ in 0..3 {
            results.push(task_result("a", 0, 0));
        }
        results.push(task_result("a", 0, 0));
        results.push(task_result("a", 0, 1)); // one trial has a false positive
        let pass_k = compute_pass_k(&results).expect("uniform k=5 reports pass^k");
        assert_eq!(
            pass_k.score.successes, 0,
            "one FP trial fails the whole task"
        );
        assert_eq!(pass_k.score.n, 1);
    }

    #[test]
    fn pass_k_is_none_for_uneven_trial_counts() {
        let results = vec![
            task_result("a", 0, 0),
            task_result("a", 0, 0),
            task_result("b", 0, 0),
        ];
        assert!(
            compute_pass_k(&results).is_none(),
            "task a has 2 trials, task b has 1 — no single k to report"
        );
    }

    #[test]
    fn pass_k_is_none_for_a_single_trial_per_task() {
        let results = vec![task_result("a", 0, 0), task_result("b", 0, 0)];
        assert!(
            compute_pass_k(&results).is_none(),
            "k=1 has no repetition to measure consistency over"
        );
    }
}
