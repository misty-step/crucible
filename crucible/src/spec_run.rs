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

use anyhow::Context;
use crucible_core::{
    findings_from_artifact, schema_valid, to_key_findings, AggregationMethod, CerberusReceiptTask,
    CorpusSpec, Defect, EvalSpec, ExpectedKey, IntervalMethod, KeyFinding, RunnerKind, RunnerSpec,
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

fn grade_key_recall_task(findings: &[KeyFinding], expected: &ExpectedKey) -> KeyRecallTaskScore {
    let grade = score_against_expected_key(findings, expected);
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

fn score_against_expected_key(findings: &[KeyFinding], expected: &ExpectedKey) -> SpanGrade {
    let mut matched_flags = vec![false; expected.defects.len()];
    let mut matched_ids = Vec::new();
    let mut false_positives = 0u64;

    for finding in findings {
        let hit = expected
            .defects
            .iter()
            .enumerate()
            .position(|(i, defect)| !matched_flags[i] && defect_matches(finding, defect));
        match hit {
            Some(i) => {
                matched_flags[i] = true;
                matched_ids.push(expected.defects[i].id.clone());
            }
            None => false_positives += 1,
        }
    }

    let missed_ids = expected
        .defects
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_flags[*i])
        .map(|(_, defect)| defect.id.clone())
        .collect();

    SpanGrade {
        matched_ids,
        missed_ids,
        false_positives,
    }
}

fn defect_matches(finding: &KeyFinding, defect: &Defect) -> bool {
    finding.file == defect.file
        && finding.category == defect.category
        && defect.line_start <= finding.line
        && finding.line <= defect.line_end
        && severity_matches(finding.severity.as_str(), defect.severity.as_deref())
}

fn severity_matches(candidate: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    match (severity_rank(candidate), severity_rank(expected)) {
        (Some(candidate), Some(expected)) => candidate <= expected,
        _ => false,
    }
}

fn severity_rank(label: &str) -> Option<u8> {
    match label {
        "blocking" => Some(0),
        "serious" => Some(1),
        "minor" => Some(2),
        _ => None,
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
    grade: SpanGrade,
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

#[derive(Debug, PartialEq, Eq)]
struct SpanGrade {
    matched_ids: Vec<String>,
    missed_ids: Vec<String>,
    false_positives: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

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
            defects: vec![Defect {
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
        let grade = score_against_expected_key(&[just_outside], &expected);
        assert_eq!(
            grade,
            SpanGrade {
                matched_ids: Vec::new(),
                missed_ids: vec!["d1".to_string()],
                false_positives: 1,
            }
        );
    }
}
