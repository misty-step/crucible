//! Declared `EvalSpec` execution for the first real benchmark surface.
//!
//! This is intentionally narrower than a general runner framework. It executes
//! the first load-bearing spec shape Crucible needs now: key recall over
//! Daedalus PR-review `trials.jsonl` corpora and fresh Cerberus review artifacts
//! handed off with receipt bundles. New runner families should earn their own
//! explicit branch in the spec schema and here.

use std::collections::HashSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use crucible_core::{
    findings_from_artifact, schema_valid, to_key_findings, AgenticJudgeConfig, AgenticJudgeTask,
    AggregationMethod, CerberusReceiptTask, CorpusSpec, EvalSpec, ExpectedKey, GraderKind,
    IntervalMethod, KeyFinding, ModelProvider, PromptBenchmarkTask, PromptExpectation,
    PromptModelConfig, RunnerKind, RunnerSpec,
};
use serde::{Deserialize, Serialize};

use crate::eval_run::{EvalReport, RunReport, Score, RUN_REPORT_SCHEMA};
use crate::wilson_score;

/// Execute a declared eval spec and write a `crucible.run_report.v1` plus
/// runner-specific evidence under `out`.
pub fn run(spec_path: &Path, out: Option<&Path>) -> anyhow::Result<RunReport> {
    let spec = load_spec(spec_path)?;
    let out = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new("runs/local").join(spec_id(&spec, spec_path)));
    std::fs::create_dir_all(&out)
        .with_context(|| format!("creating run output directory {}", out.display()))?;

    let runner = spec.runner.as_ref().with_context(|| {
        format!(
            "spec {} is definition-only: it has no executable runner declaration",
            spec_path.display()
        )
    })?;
    let eval = run_runner(&spec, runner, spec_path, &out)?;
    let report = RunReport {
        schema_version: RUN_REPORT_SCHEMA,
        output_dir: out.display().to_string(),
        evals: vec![eval],
    };
    write_json(&out.join("run-report.json"), &report)?;
    Ok(report)
}

fn load_spec(spec_path: &Path) -> anyhow::Result<EvalSpec> {
    let bytes = std::fs::read(spec_path)
        .with_context(|| format!("reading eval spec {}", spec_path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {} as a Crucible EvalSpec", spec_path.display()))
}

fn run_runner(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
) -> anyhow::Result<EvalReport> {
    match runner.kind {
        RunnerKind::KeyRecall => run_key_recall(spec, runner, spec_path, out),
        RunnerKind::PromptBenchmark => run_prompt_benchmark(spec, runner, spec_path, out),
        RunnerKind::AgenticJudge => run_agentic_judge(spec, runner, spec_path, out),
    }
}

fn run_key_recall(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
) -> anyhow::Result<EvalReport> {
    if spec.aggregation != AggregationMethod::Proportion {
        anyhow::bail!(
            "key_recall runner requires aggregation=proportion, got {:?}",
            spec.aggregation
        );
    }
    if spec.uncertainty.method != IntervalMethod::Wilson {
        anyhow::bail!(
            "key_recall runner requires uncertainty.method=wilson, got {:?}",
            spec.uncertainty.method
        );
    }

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
        CorpusSpec::PromptBenchmark { .. } | CorpusSpec::AgenticJudge { .. } => {
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
    let arena_dir = resolve_spec_path(spec_path, arena_dir);
    let trials_jsonl = resolve_spec_path(spec_path, trials_jsonl);
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
                "Selected {} Daedalus trial(s) for candidate {:?} and graded them against Harbor scorer keys.",
                selected_trial_count, candidate_id
            ),
        ],
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

fn run_prompt_benchmark(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
) -> anyhow::Result<EvalReport> {
    if spec.aggregation != AggregationMethod::Proportion {
        anyhow::bail!(
            "prompt_benchmark runner requires aggregation=proportion, got {:?}",
            spec.aggregation
        );
    }
    if spec.uncertainty.method != IntervalMethod::Wilson {
        anyhow::bail!(
            "prompt_benchmark runner requires uncertainty.method=wilson, got {:?}",
            spec.uncertainty.method
        );
    }

    let CorpusSpec::PromptBenchmark { config, tasks } = &runner.corpus else {
        anyhow::bail!("prompt_benchmark runner requires corpus.source=prompt_benchmark");
    };
    let client = OpenRouterClient::from_config(config)?;
    run_prompt_benchmark_with_client(spec, runner, spec_path, out, config, tasks, &client)
}

fn run_prompt_benchmark_with_client(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
    config: &PromptModelConfig,
    tasks: &[PromptBenchmarkTask],
    model_client: &dyn ModelClient,
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

    let mut task_results = Vec::new();
    let mut passed = 0u64;

    for task in tasks {
        let started = Instant::now();
        let response = model_client.complete(ModelRequest {
            model: &config.model,
            system_prompt: &config.system_prompt,
            user_prompt: &task.prompt,
            max_output_units: config.max_output_units,
            temperature: config.temperature,
        })?;
        let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let task_passed = prompt_expectation_passes(&response.output, &task.expectation);
        if task_passed {
            passed += 1;
        }

        task_results.push(PromptTaskResult {
            task_id: task.task_id.clone(),
            prompt_hash: stable_hash(&[&config.system_prompt, &task.prompt]),
            rubric_hash: stable_hash(&[
                expectation_kind(&task.expectation),
                expectation_value(&task.expectation),
            ]),
            expectation: task.expectation.clone(),
            passed: task_passed,
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

/// Judge protocol suffix appended to every judge call's system prompt so the
/// response is parseable regardless of what the operator wrote in
/// `judge_prompt`: exactly one `VERDICT: PASS` or `VERDICT: FAIL` line.
const JUDGE_VERDICT_PROTOCOL: &str = "\n\nRespond with exactly one line in the form `VERDICT: PASS` or `VERDICT: FAIL`, followed by one sentence of reasoning on the next line. Do not rubber-stamp: a candidate that fails the rubric must get VERDICT: FAIL even if it is close.";

fn run_agentic_judge(
    spec: &EvalSpec,
    runner: &RunnerSpec,
    spec_path: &Path,
    out: &Path,
) -> anyhow::Result<EvalReport> {
    if spec.aggregation != AggregationMethod::Proportion {
        anyhow::bail!(
            "agentic_judge runner requires aggregation=proportion, got {:?}",
            spec.aggregation
        );
    }
    if spec.uncertainty.method != IntervalMethod::Wilson {
        anyhow::bail!(
            "agentic_judge runner requires uncertainty.method=wilson, got {:?}",
            spec.uncertainty.method
        );
    }
    // GraderKind::Agentic constructed for real (backlog 012): the spec must
    // name an agentic grader before this runner will make a model call.
    if !spec
        .graders
        .graders
        .iter()
        .any(|grader| grader.kind == GraderKind::Agentic)
    {
        anyhow::bail!(
            "agentic_judge runner requires an Agentic grader declared in graders.graders"
        );
    }

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
    let mut task_results = Vec::new();
    let mut scored_successes = 0u64;
    let mut scored_n = 0u64;
    let mut canary_notes = Vec::new();

    for task in tasks {
        let user_prompt = format!(
            "Rubric:\n{}\n\nCandidate output:\n{}",
            task.rubric, task.candidate
        );
        let started = Instant::now();
        let response = model_client.complete(ModelRequest {
            model: &config.model,
            system_prompt: &judge_system_prompt,
            user_prompt: &user_prompt,
            max_output_units: None,
            temperature: config.temperature,
        })?;
        let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let verdict = parse_judge_verdict(&response.output).with_context(|| {
            format!(
                "agentic judge task {:?} returned an unparseable verdict",
                task.task_id
            )
        })?;

        match task.expected_pass {
            Some(expected) if expected != verdict => {
                if task.refuse_on_mismatch {
                    anyhow::bail!(
                        "judge-gaming guard tripped on task {:?}: expected verdict {expected} but the judge said {verdict}; refusing to trust this run",
                        task.task_id
                    );
                }
                canary_notes.push(format!(
                    "calibration task {:?} disagreed with the judge (expected {expected}, got {verdict}) but did not refuse the run.",
                    task.task_id
                ));
            }
            Some(_) => {
                canary_notes.push(format!(
                    "calibration task {:?} matched its expected verdict.",
                    task.task_id
                ));
            }
            None => {
                scored_n += 1;
                if verdict {
                    scored_successes += 1;
                }
            }
        }

        task_results.push(AgenticJudgeTaskResult {
            task_id: task.task_id.clone(),
            prompt_hash: stable_hash(&[&judge_system_prompt, &user_prompt]),
            rubric_hash: stable_hash(&[&task.rubric]),
            expected_pass: task.expected_pass,
            passed: verdict,
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
            system_prompt_hash: stable_hash(&[&judge_system_prompt]),
            score: &score,
            totals: PromptTotals {
                tasks: scored_n,
                passed: scored_successes,
                failed: scored_n - scored_successes,
            },
            tasks: &task_results,
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
    notes.extend(canary_notes);

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
        ],
        notes,
    })
}

/// Parse a judge response for the `VERDICT: PASS`/`VERDICT: FAIL` line the
/// judge protocol requires. Refuses to guess: an ambiguous or missing verdict
/// is an error, not a default pass or fail.
fn parse_judge_verdict(output: &str) -> anyhow::Result<bool> {
    let upper = output.to_uppercase();
    let pass = upper.contains("VERDICT: PASS") || upper.contains("VERDICT:PASS");
    let fail = upper.contains("VERDICT: FAIL") || upper.contains("VERDICT:FAIL");
    match (pass, fail) {
        (true, false) => Ok(true),
        (false, true) => Ok(false),
        (false, false) => {
            anyhow::bail!("judge response had no VERDICT: PASS/FAIL line: {output:?}")
        }
        (true, true) => {
            anyhow::bail!("judge response had both VERDICT: PASS and VERDICT: FAIL: {output:?}")
        }
    }
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

fn resolve_spec_path(spec_path: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        spec_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
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
        serde_json::Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                if let Some(text) = part.get("text").and_then(serde_json::Value::as_str) {
                    out.push_str(text);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }
        _ => None,
    }
}

fn prompt_expectation_passes(output: &str, expectation: &PromptExpectation) -> bool {
    match expectation {
        PromptExpectation::Exact { value } => output.trim() == value.trim(),
        PromptExpectation::Contains { value } => output.contains(value),
    }
}

fn expectation_kind(expectation: &PromptExpectation) -> &'static str {
    match expectation {
        PromptExpectation::Exact { .. } => "exact",
        PromptExpectation::Contains { .. } => "contains",
    }
}

fn expectation_value(expectation: &PromptExpectation) -> &str {
    match expectation {
        PromptExpectation::Exact { value } | PromptExpectation::Contains { value } => value,
    }
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
}

#[derive(Debug, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
enum CorpusEvidence {
    DaedalusTrials {
        arena_dir: String,
        trials_jsonl: String,
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
    prompt_hash: String,
    rubric_hash: String,
    expectation: PromptExpectation,
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
struct AgenticJudgeEvidence<'a> {
    schema_version: &'static str,
    spec_id: String,
    spec: String,
    runner: RunnerKind,
    provider: ModelProvider,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<u32>,
    system_prompt_hash: String,
    score: &'a Score,
    totals: PromptTotals,
    tasks: &'a [AgenticJudgeTaskResult],
}

#[derive(Debug, Serialize)]
struct AgenticJudgeTaskResult {
    task_id: String,
    prompt_hash: String,
    rubric_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_pass: Option<bool>,
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
    /// distinct verdicts.
    struct QueuedModelClient {
        outputs: std::cell::RefCell<std::collections::VecDeque<&'static str>>,
    }

    impl QueuedModelClient {
        fn new(outputs: Vec<&'static str>) -> Self {
            Self {
                outputs: std::cell::RefCell::new(outputs.into_iter().collect()),
            }
        }
    }

    impl ModelClient for QueuedModelClient {
        fn complete(&self, request: ModelRequest<'_>) -> anyhow::Result<ModelResponse> {
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
            task: "prompt-smoke".to_string(),
            inputs: String::new(),
            outputs: String::new(),
            fixtures: Vec::new(),
            graders: crucible_core::GraderManifest::default(),
            baselines: Vec::new(),
            aggregation: AggregationMethod::Proportion,
            uncertainty: crucible_core::UncertaintyRule::default(),
            decision: String::new(),
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
                },
                tasks: vec![PromptBenchmarkTask {
                    task_id: "exact".to_string(),
                    prompt: "Reply with exactly: crucible-smoke".to_string(),
                    expectation: PromptExpectation::Exact {
                        value: "crucible-smoke".to_string(),
                    },
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
        let evidence = std::fs::read_to_string(temp.join("prompt-run.json"))
            .expect("prompt evidence is written");
        let evidence: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        assert_eq!(
            evidence["schema_version"],
            "crucible.prompt_run_evidence.v1"
        );
        assert_eq!(evidence["tasks"][0]["output"], "crucible-smoke");
        assert_eq!(evidence["tasks"][0]["passed"], true);
        assert_eq!(evidence["tasks"][0]["total_tokens"], 10);
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
            },
            AgenticJudgeTask {
                task_id: "canary".to_string(),
                candidate: "This answer is nonsense and ignores the question.".to_string(),
                rubric: "The answer must be correct and well-reasoned.".to_string(),
                expected_pass: Some(false),
                refuse_on_mismatch: true,
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
            "VERDICT: PASS\nThe answer is correct.",
            "VERDICT: FAIL\nThe answer does not address the rubric.",
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
            },
            AgenticJudgeTask {
                task_id: "canary".to_string(),
                candidate: "This answer is nonsense and ignores the question.".to_string(),
                rubric: "The answer must be correct and well-reasoned.".to_string(),
                expected_pass: Some(false),
                refuse_on_mismatch: true,
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
            "VERDICT: PASS\nLooks fine.",
            "VERDICT: PASS\nLooks fine.",
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
        }];
        let runner = RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge { config, tasks },
        };

        let err = run_runner(&spec, &runner, &spec_path, &temp)
            .expect_err("a spec without a declared Agentic grader must refuse to run");
        assert!(
            err.to_string().contains("requires an Agentic grader"),
            "error names the missing grader declaration: {err}"
        );
    }

    #[test]
    fn parse_judge_verdict_refuses_ambiguous_output() {
        assert!(parse_judge_verdict("VERDICT: PASS\nreason").unwrap());
        assert!(!parse_judge_verdict("VERDICT: FAIL\nreason").unwrap());
        assert!(parse_judge_verdict("no verdict here").is_err());
        assert!(parse_judge_verdict("VERDICT: PASS and also VERDICT: FAIL").is_err());
    }
}
