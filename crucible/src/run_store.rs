//! SQLite run ledger for Crucible-owned benchmark executions.
//!
//! The ledger is deliberately boring: one invocation row, one row per eval
//! result, artifact pointers, and runner-specific task rows where Crucible knows
//! how to index them. Full JSON copies stay with each row so future
//! `RunRecord`/`EvaluationCard` materialization can migrate forward without
//! reparsing chat or relying on a loose artifact still existing.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use crucible_core::{
    EvalSpec, EvaluationCard, FixtureRef, McnemarOutcome, PairedComparison, Provenance, RunRecord,
    RunScore, EVALUATION_CARD_SCHEMA, RUN_RECORD_SCHEMA,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::eval_run::{EvalReport, RunReport};

/// Default local run ledger path. The whole `runs/` tree is gitignored because
/// real eval runs may contain proprietary diffs and raw model output.
pub const DEFAULT_DB_PATH: &str = "runs/local/crucible-runs.sqlite";

/// Default significance threshold for the paired McNemar verdict in
/// [`compare_configs`].
pub const DEFAULT_ALPHA: f64 = 0.05;

const RUN_STORE_SCHEMA: &str = "crucible.run_store.v1";
static INVOCATION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize)]
pub struct PersistedReport {
    pub schema_version: &'static str,
    pub db: String,
    pub invocation_id: String,
    pub output_dir: String,
    pub run_report: String,
    pub run_records: usize,
    pub prompt_task_results: usize,
}

/// Filter for [`list_runs`]. `None` fields are unconstrained.
#[derive(Debug, Default, Clone, Copy)]
pub struct RunListFilter<'a> {
    pub benchmark: Option<&'a str>,
    pub config: Option<&'a str>,
    pub model: Option<&'a str>,
    pub since_unix_ms: Option<i64>,
    pub until_unix_ms: Option<i64>,
    /// Cap on the number of rows returned. `None` is unconstrained (every
    /// matching row comes back, the historical no-pagination behavior) —
    /// callers that want a bounded page set this explicitly.
    pub limit: Option<i64>,
    /// Rows to skip before the first returned row, applied after `ORDER BY`.
    /// Ignored (treated as 0) when `limit` is `None`.
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RunList {
    pub schema_version: &'static str,
    pub db: String,
    pub benchmark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_unix_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_unix_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    pub runs: Vec<StoredRun>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredRun {
    pub run_id: String,
    pub invocation_id: String,
    pub benchmark_id: String,
    pub title: String,
    pub runner_kind: String,
    pub config_id: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at_unix_ms: i64,
    pub output_dir: String,
    pub run_report: String,
    pub evidence_path: Option<String>,
    pub spec_path: Option<String>,
    pub score_metric: String,
    pub successes: u64,
    pub n: u64,
    pub point: Option<f64>,
    pub lower: f64,
    pub upper: f64,
    pub confidence: f64,
    pub method: String,
}

#[derive(Debug, Serialize)]
pub struct RunDetail {
    pub schema_version: &'static str,
    pub db: String,
    pub run: StoredRun,
    pub artifacts: Vec<StoredArtifact>,
    pub prompt_tasks: Vec<StoredPromptTask>,
    pub run_record: Option<Value>,
    pub evaluation_card: Option<Value>,
    pub eval_json: Value,
}

#[derive(Debug, Serialize)]
pub struct StoredArtifact {
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct StoredPromptTask {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    pub passed: bool,
    pub latency_ms: Option<u64>,
    pub response_id: Option<String>,
    pub requested_model: Option<String>,
    pub response_model: Option<String>,
    pub prompt_hash: Option<String>,
    pub rubric_hash: Option<String>,
    #[serde(rename = "prompt_tokens")]
    pub input_units: Option<u64>,
    #[serde(rename = "completion_tokens")]
    pub output_units: Option<u64>,
    #[serde(rename = "total_tokens")]
    pub total_units: Option<u64>,
    pub cost_usd: Option<f64>,
    pub output_text: Option<String>,
    pub evidence_json: Value,
}

#[derive(Debug, Serialize)]
pub struct ConfigComparison {
    pub schema_version: &'static str,
    pub db: String,
    pub benchmark: String,
    pub left_query: String,
    pub right_query: String,
    pub left: StoredRun,
    pub right: StoredRun,
    pub delta_point: Option<f64>,
    /// Prompt task ids present in both the left and right run's task rows.
    /// `0` when either run has no indexed prompt tasks or the two runs share
    /// no task id — the comparison then falls back to the unpaired
    /// descriptive delta.
    pub common_tasks: usize,
    /// The paired McNemar outcome over `common_tasks`, present only when
    /// `common_tasks > 0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired: Option<McnemarOutcome>,
    pub class_breakdowns: Vec<ClassComparison>,
    pub comparison_kind: &'static str,
    pub note: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassComparison {
    pub class: String,
    pub left_successes: u64,
    pub left_n: u64,
    pub left_point: Option<f64>,
    pub right_successes: u64,
    pub right_n: u64,
    pub right_point: Option<f64>,
    pub delta_point: Option<f64>,
    pub common_tasks: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired: Option<McnemarOutcome>,
}

#[derive(Debug, Default)]
struct EvidenceMetadata {
    runner_kind: Option<String>,
    config_id: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    evidence_path: Option<String>,
    spec_path: Option<String>,
    temperature: Option<f64>,
    max_output_units: Option<u64>,
    prompt_tasks: Vec<PromptTaskInsert>,
}

#[derive(Debug, Clone)]
struct PromptTaskInsert {
    task_id: String,
    class: Option<String>,
    passed: bool,
    latency_ms: Option<u64>,
    response_id: Option<String>,
    requested_model: Option<String>,
    response_model: Option<String>,
    prompt_hash: Option<String>,
    rubric_hash: Option<String>,
    input_units: Option<u64>,
    output_units: Option<u64>,
    total_units: Option<u64>,
    cost_usd: Option<f64>,
    output_text: Option<String>,
    evidence_json: String,
}

/// Persist a run report and all recognized evidence into the SQLite ledger.
pub fn persist_report(db_path: &Path, report: &RunReport) -> Result<PersistedReport> {
    validate_db_write_path(db_path)?;
    let mut conn = open_initialized(db_path)?;
    let now_ms = now_unix_ms()?;
    let invocation_id = new_invocation_id(now_ms);
    let run_report_path = Path::new(&report.output_dir)
        .join("run-report.json")
        .display()
        .to_string();
    let report_json = serde_json::to_string(report).context("serializing run report")?;

    let tx = conn
        .transaction()
        .context("opening run-store transaction")?;
    tx.execute(
        "INSERT INTO invocations (
            invocation_id, created_at_unix_ms, output_dir, run_report_path,
            report_schema_version, report_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            invocation_id,
            now_ms,
            report.output_dir,
            run_report_path,
            report.schema_version,
            report_json
        ],
    )
    .context("inserting run invocation")?;

    let mut prompt_task_results = 0usize;
    for (index, eval) in report.evals.iter().enumerate() {
        let metadata = extract_metadata(eval)?;
        let run_id = format!("{}:{}", invocation_id, eval.id);
        let eval_json = serde_json::to_string(eval).context("serializing eval report")?;
        let runner_kind = metadata
            .runner_kind
            .clone()
            .unwrap_or_else(|| "built_in".to_string());
        let config_id = metadata
            .config_id
            .clone()
            .unwrap_or_else(|| "built-in".to_string());
        let (run_record, evaluation_card) = materialize_run_record(&MaterializeInput {
            eval,
            metadata: &metadata,
            run_id: &run_id,
            runner_kind: &runner_kind,
            config_id: &config_id,
            now_ms,
            output_dir: &report.output_dir,
            run_report_path: &run_report_path,
        })?;
        let run_record_json =
            serde_json::to_string(&run_record).context("serializing run record")?;
        let evaluation_card_json =
            serde_json::to_string(&evaluation_card).context("serializing evaluation card")?;

        tx.execute(
            "INSERT INTO run_records (
                run_id, invocation_id, ordinal, benchmark_id, title, runner_kind,
                config_id, provider, model, created_at_unix_ms, output_dir,
                run_report_path, evidence_path, spec_path, score_metric, successes,
                n, point, lower, upper, confidence, score_method, eval_json
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23
            )",
            params![
                run_id,
                invocation_id,
                to_i64(index)?,
                eval.id,
                eval.title,
                runner_kind,
                config_id,
                metadata.provider,
                metadata.model,
                now_ms,
                report.output_dir,
                run_report_path,
                metadata.evidence_path,
                metadata.spec_path,
                eval.score.metric,
                to_i64(eval.score.successes)?,
                to_i64(eval.score.n)?,
                eval.score.point,
                eval.score.lower,
                eval.score.upper,
                eval.score.confidence,
                eval.score.method,
                eval_json
            ],
        )
        .with_context(|| format!("inserting run record for {}", eval.id))?;

        for artifact in &eval.artifacts {
            tx.execute(
                "INSERT INTO run_artifacts (run_id, path, kind)
                 VALUES (?1, ?2, ?3)",
                params![run_id, artifact, artifact_kind(artifact)],
            )
            .with_context(|| format!("inserting artifact pointer {artifact}"))?;
        }

        tx.execute(
            "INSERT INTO run_record_materializations (
                run_id, run_record_schema_version, run_record_json,
                evaluation_card_schema_version, evaluation_card_json
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                run_id,
                run_record.schema_version,
                run_record_json,
                evaluation_card.schema_version,
                evaluation_card_json
            ],
        )
        .with_context(|| format!("inserting durable run record for {}", eval.id))?;

        for task in metadata.prompt_tasks {
            tx.execute(
                "INSERT INTO prompt_task_results (
                    run_id, task_id, task_class, passed, latency_ms, response_id,
                    requested_model, response_model, prompt_hash, rubric_hash,
                    prompt_tokens, completion_tokens, total_tokens, cost_usd,
                    output_text, evidence_json
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    ?14, ?15, ?16
                )",
                params![
                    run_id,
                    task.task_id,
                    task.class,
                    if task.passed { 1i64 } else { 0i64 },
                    opt_i64(task.latency_ms)?,
                    task.response_id,
                    task.requested_model,
                    task.response_model,
                    task.prompt_hash,
                    task.rubric_hash,
                    opt_i64(task.input_units)?,
                    opt_i64(task.output_units)?,
                    opt_i64(task.total_units)?,
                    task.cost_usd,
                    task.output_text,
                    task.evidence_json
                ],
            )
            .context("inserting prompt task result")?;
            prompt_task_results += 1;
        }
    }

    tx.commit().context("committing run-store transaction")?;
    Ok(PersistedReport {
        schema_version: RUN_STORE_SCHEMA,
        db: db_path.display().to_string(),
        invocation_id,
        output_dir: report.output_dir.clone(),
        run_report: run_report_path,
        run_records: report.evals.len(),
        prompt_task_results,
    })
}

fn validate_db_write_path(db_path: &Path) -> Result<()> {
    let cwd = lexical_normalize(&std::env::current_dir().context("reading current directory")?);
    let absolute = if db_path.is_absolute() {
        lexical_normalize(db_path)
    } else {
        lexical_normalize(&cwd.join(db_path))
    };
    let ignored_runs = lexical_normalize(&cwd.join("runs"));

    if absolute.starts_with(&cwd) && !absolute.starts_with(&ignored_runs) {
        anyhow::bail!(
            "run database path inside this checkout must live under gitignored runs/; got {}",
            db_path.display()
        );
    }
    Ok(())
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn list_runs(db_path: &Path, filter: RunListFilter<'_>) -> Result<RunList> {
    let conn = open_initialized(db_path)?;
    // SQLite treats `LIMIT -1` as "no limit" while still honoring OFFSET, so a
    // `None` limit stays a true full scan (unchanged historical behavior) and
    // a `Some` limit bounds the query at the SQL layer rather than filtering
    // a fully-materialized Rust `Vec` after the fact.
    let mut stmt = conn
        .prepare(
            "SELECT run_id, invocation_id, benchmark_id, title, runner_kind,
                config_id, provider, model, created_at_unix_ms, output_dir,
                run_report_path, evidence_path, spec_path, score_metric,
                successes, n, point, lower, upper, confidence, score_method
             FROM run_records
             WHERE (?1 IS NULL OR benchmark_id = ?1)
               AND (?2 IS NULL OR config_id = ?2)
               AND (?3 IS NULL OR model = ?3)
               AND (?4 IS NULL OR created_at_unix_ms >= ?4)
               AND (?5 IS NULL OR created_at_unix_ms <= ?5)
             ORDER BY created_at_unix_ms DESC, run_id DESC
             LIMIT COALESCE(?6, -1) OFFSET COALESCE(?7, 0)",
        )
        .context("preparing run list query")?;
    let rows = stmt
        .query_map(
            params![
                filter.benchmark,
                filter.config,
                filter.model,
                filter.since_unix_ms,
                filter.until_unix_ms,
                filter.limit,
                filter.offset,
            ],
            row_to_stored_run,
        )
        .context("querying run list")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("reading run list rows")?;

    Ok(RunList {
        schema_version: RUN_STORE_SCHEMA,
        db: db_path.display().to_string(),
        benchmark: filter.benchmark.map(str::to_string),
        config: filter.config.map(str::to_string),
        model: filter.model.map(str::to_string),
        since_unix_ms: filter.since_unix_ms,
        until_unix_ms: filter.until_unix_ms,
        limit: filter.limit,
        offset: filter.offset,
        runs: rows,
    })
}

pub fn show_run(db_path: &Path, run_id: &str) -> Result<RunDetail> {
    let conn = open_initialized(db_path)?;
    let run = conn
        .query_row(
            "SELECT run_id, invocation_id, benchmark_id, title, runner_kind,
                config_id, provider, model, created_at_unix_ms, output_dir,
                run_report_path, evidence_path, spec_path, score_metric,
                successes, n, point, lower, upper, confidence, score_method
             FROM run_records
             WHERE run_id = ?1",
            params![run_id],
            row_to_stored_run,
        )
        .optional()
        .context("querying run detail")?
        .with_context(|| format!("run id {run_id:?} not found"))?;

    let eval_json: String = conn
        .query_row(
            "SELECT eval_json FROM run_records WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .context("loading stored eval JSON")?;
    let eval_json = serde_json::from_str(&eval_json).context("parsing stored eval JSON")?;

    let artifacts = query_artifacts(&conn, run_id)?;
    let prompt_tasks = query_prompt_tasks(&conn, run_id)?;
    let materialization = query_materialization(&conn, run_id)?;

    Ok(RunDetail {
        schema_version: RUN_STORE_SCHEMA,
        db: db_path.display().to_string(),
        run,
        artifacts,
        prompt_tasks,
        run_record: materialization
            .as_ref()
            .map(|materialization| materialization.run_record.clone()),
        evaluation_card: materialization.map(|materialization| materialization.evaluation_card),
        eval_json,
    })
}

/// Compare the latest stored run per config/model under one benchmark.
///
/// When both runs carry indexed prompt task rows that share at least one task
/// id, the comparison is a paired [`McnemarOutcome`] over those shared tasks
/// (backlog 003's noise-floor discipline: the discordant pairs are the only
/// ones carrying information). Otherwise it falls back to the unpaired
/// descriptive delta between each run's own point estimate — the same
/// behavior as before this comparison learned to pair.
pub fn compare_configs(
    db_path: &Path,
    benchmark: &str,
    left: &str,
    right: &str,
    alpha: f64,
) -> Result<ConfigComparison> {
    let conn = open_initialized(db_path)?;
    let left_run = latest_for_config(&conn, benchmark, left).with_context(|| {
        format!("no run found for benchmark {benchmark:?} and config/model {left:?}")
    })?;
    let right_run = latest_for_config(&conn, benchmark, right).with_context(|| {
        format!("no run found for benchmark {benchmark:?} and config/model {right:?}")
    })?;
    let delta_point = match (left_run.point, right_run.point) {
        (Some(left), Some(right)) => Some(right - left),
        _ => None,
    };

    let left_tasks = query_prompt_tasks(&conn, &left_run.run_id)?;
    let right_tasks = query_prompt_tasks(&conn, &right_run.run_id)?;
    let (paired, common_tasks) = match paired_mcnemar(&left_tasks, &right_tasks, alpha) {
        Some((outcome, n)) => (Some(outcome), n),
        None => (None, 0),
    };
    let class_breakdowns = compare_by_class(&left_tasks, &right_tasks, alpha);

    let (comparison_kind, note): (&'static str, &'static str) = if paired.is_some() {
        (
            "paired_mcnemar",
            "Paired McNemar comparison over per-task outcomes common to both runs (prompt tasks or pass^k task consistency); see paired.verdict for the noise-floor decision.",
        )
    } else {
        (
            "latest_unpaired_descriptive_delta",
            "This compares the latest matching run per config/model and does not assert statistical significance.",
        )
    };

    Ok(ConfigComparison {
        schema_version: RUN_STORE_SCHEMA,
        db: db_path.display().to_string(),
        benchmark: benchmark.to_string(),
        left_query: left.to_string(),
        right_query: right.to_string(),
        left: left_run,
        right: right_run,
        delta_point,
        common_tasks,
        paired,
        class_breakdowns,
        comparison_kind,
        note,
    })
}

/// McNemar outcome over the prompt task ids common to both sides, or `None`
/// when either side has no indexed prompt tasks or the two share none.
fn paired_mcnemar(
    left: &[StoredPromptTask],
    right: &[StoredPromptTask],
    alpha: f64,
) -> Option<(McnemarOutcome, usize)> {
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let right_by_task: HashMap<&str, bool> = right
        .iter()
        .map(|task| (task.task_id.as_str(), task.passed))
        .collect();

    let mut b: u64 = 0; // left passed, right failed
    let mut c: u64 = 0; // left failed, right passed
    let mut common = 0usize;
    for task in left {
        let Some(&right_passed) = right_by_task.get(task.task_id.as_str()) else {
            continue;
        };
        common += 1;
        match (task.passed, right_passed) {
            (true, false) => b += 1,
            (false, true) => c += 1,
            _ => {}
        }
    }
    if common == 0 {
        return None;
    }

    let cmp = PairedComparison::mcnemar(b, c);
    Some((
        McnemarOutcome {
            b: cmp.b,
            c: cmp.c,
            statistic: cmp.statistic,
            p_value: cmp.p_value,
            verdict: cmp.verdict(alpha),
        },
        common,
    ))
}

fn compare_by_class(
    left: &[StoredPromptTask],
    right: &[StoredPromptTask],
    alpha: f64,
) -> Vec<ClassComparison> {
    let mut classes: BTreeMap<String, (Vec<&StoredPromptTask>, Vec<&StoredPromptTask>)> =
        BTreeMap::new();
    for task in left {
        classes
            .entry(task_class(task).to_string())
            .or_default()
            .0
            .push(task);
    }
    for task in right {
        classes
            .entry(task_class(task).to_string())
            .or_default()
            .1
            .push(task);
    }

    classes
        .into_iter()
        .map(|(class, (left_tasks, right_tasks))| {
            let left_successes = left_tasks.iter().filter(|task| task.passed).count() as u64;
            let right_successes = right_tasks.iter().filter(|task| task.passed).count() as u64;
            let left_n = left_tasks.len() as u64;
            let right_n = right_tasks.len() as u64;
            let left_point = proportion_point(left_successes, left_n);
            let right_point = proportion_point(right_successes, right_n);
            let delta_point = match (left_point, right_point) {
                (Some(left), Some(right)) => Some(right - left),
                _ => None,
            };
            let (paired, common_tasks) = match paired_mcnemar_refs(&left_tasks, &right_tasks, alpha)
            {
                Some((outcome, n)) => (Some(outcome), n),
                None => (None, 0),
            };
            ClassComparison {
                class,
                left_successes,
                left_n,
                left_point,
                right_successes,
                right_n,
                right_point,
                delta_point,
                common_tasks,
                paired,
            }
        })
        .collect()
}

fn task_class(task: &StoredPromptTask) -> &str {
    task.class.as_deref().unwrap_or("unclassified")
}

fn proportion_point(successes: u64, n: u64) -> Option<f64> {
    if n == 0 {
        None
    } else {
        Some(successes as f64 / n as f64)
    }
}

fn paired_mcnemar_refs(
    left: &[&StoredPromptTask],
    right: &[&StoredPromptTask],
    alpha: f64,
) -> Option<(McnemarOutcome, usize)> {
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let right_by_task: HashMap<&str, bool> = right
        .iter()
        .map(|task| (task.task_id.as_str(), task.passed))
        .collect();

    let mut b: u64 = 0;
    let mut c: u64 = 0;
    let mut common = 0usize;
    for task in left {
        let Some(&right_passed) = right_by_task.get(task.task_id.as_str()) else {
            continue;
        };
        common += 1;
        match (task.passed, right_passed) {
            (true, false) => b += 1,
            (false, true) => c += 1,
            _ => {}
        }
    }
    if common == 0 {
        return None;
    }

    let cmp = PairedComparison::mcnemar(b, c);
    Some((
        McnemarOutcome {
            b: cmp.b,
            c: cmp.c,
            statistic: cmp.statistic,
            p_value: cmp.p_value,
            verdict: cmp.verdict(alpha),
        },
        common,
    ))
}

/// How long a connection blocks-and-retries on `SQLITE_BUSY` before giving up,
/// rather than failing the instant a concurrent reader/writer holds the lock.
/// Every runner invocation, `crucible runs` query, and `serve` request opens
/// its own short-lived [`Connection`] against the same on-disk file (see
/// [`open_initialized`]), so concurrent access is routine, not exceptional,
/// once `serve`'s accept loop stops serializing requests.
const RUN_LEDGER_BUSY_TIMEOUT_MS: u64 = 5_000;

fn open_initialized(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating run database directory {}", parent.display()))?;
    }
    let conn = Connection::open(db_path)
        .with_context(|| format!("opening run database {}", db_path.display()))?;
    // Explicit, not relying on rusqlite's own internal default: self-documents
    // the contention-tolerance contract here and survives a future rusqlite
    // upgrade that might change (or drop) its implicit default.
    conn.busy_timeout(std::time::Duration::from_millis(RUN_LEDGER_BUSY_TIMEOUT_MS))
        .context("setting sqlite busy_timeout")?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        INSERT OR IGNORE INTO schema_meta (key, value)
        VALUES ('schema_version', 'crucible.run_store.v1');

        CREATE TABLE IF NOT EXISTS invocations (
            invocation_id TEXT PRIMARY KEY,
            created_at_unix_ms INTEGER NOT NULL,
            output_dir TEXT NOT NULL,
            run_report_path TEXT NOT NULL,
            report_schema_version TEXT NOT NULL,
            report_json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS run_records (
            run_id TEXT PRIMARY KEY,
            invocation_id TEXT NOT NULL REFERENCES invocations(invocation_id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL,
            benchmark_id TEXT NOT NULL,
            title TEXT NOT NULL,
            runner_kind TEXT NOT NULL,
            config_id TEXT NOT NULL,
            provider TEXT,
            model TEXT,
            created_at_unix_ms INTEGER NOT NULL,
            output_dir TEXT NOT NULL,
            run_report_path TEXT NOT NULL,
            evidence_path TEXT,
            spec_path TEXT,
            score_metric TEXT NOT NULL,
            successes INTEGER NOT NULL,
            n INTEGER NOT NULL,
            point REAL,
            lower REAL NOT NULL,
            upper REAL NOT NULL,
            confidence REAL NOT NULL,
            score_method TEXT NOT NULL,
            eval_json TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_run_records_benchmark
            ON run_records(benchmark_id, created_at_unix_ms DESC);
        CREATE INDEX IF NOT EXISTS idx_run_records_config
            ON run_records(benchmark_id, config_id, created_at_unix_ms DESC);
        CREATE INDEX IF NOT EXISTS idx_run_records_model
            ON run_records(benchmark_id, model, created_at_unix_ms DESC);

        CREATE TABLE IF NOT EXISTS run_artifacts (
            run_id TEXT NOT NULL REFERENCES run_records(run_id) ON DELETE CASCADE,
            path TEXT NOT NULL,
            kind TEXT NOT NULL,
            PRIMARY KEY (run_id, path)
        );

        CREATE TABLE IF NOT EXISTS prompt_task_results (
            run_id TEXT NOT NULL REFERENCES run_records(run_id) ON DELETE CASCADE,
            task_id TEXT NOT NULL,
            passed INTEGER NOT NULL,
            latency_ms INTEGER,
            response_id TEXT,
            requested_model TEXT,
            response_model TEXT,
            prompt_hash TEXT,
            rubric_hash TEXT,
            prompt_tokens INTEGER,
            completion_tokens INTEGER,
            total_tokens INTEGER,
            cost_usd REAL,
            output_text TEXT,
            evidence_json TEXT NOT NULL,
            PRIMARY KEY (run_id, task_id)
        );

        CREATE TABLE IF NOT EXISTS run_record_materializations (
            run_id TEXT PRIMARY KEY REFERENCES run_records(run_id) ON DELETE CASCADE,
            run_record_schema_version TEXT NOT NULL,
            run_record_json TEXT NOT NULL,
            evaluation_card_schema_version TEXT NOT NULL,
            evaluation_card_json TEXT NOT NULL
        );
        ",
    )
    .context("initializing run-store schema")?;
    ensure_prompt_task_class_column(conn)
}

fn ensure_prompt_task_class_column(conn: &Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(prompt_task_results)")
        .context("preparing prompt_task_results schema inspection")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .context("querying prompt_task_results schema")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("reading prompt_task_results schema")?;
    if !columns.iter().any(|column| column == "task_class") {
        conn.execute(
            "ALTER TABLE prompt_task_results ADD COLUMN task_class TEXT",
            [],
        )
        .context("adding prompt_task_results.task_class column")?;
    }
    Ok(())
}

struct MaterializedRecord {
    run_record: Value,
    evaluation_card: Value,
}

struct MaterializeInput<'a> {
    eval: &'a EvalReport,
    metadata: &'a EvidenceMetadata,
    run_id: &'a str,
    runner_kind: &'a str,
    config_id: &'a str,
    now_ms: i64,
    output_dir: &'a str,
    run_report_path: &'a str,
}

fn materialize_run_record(input: &MaterializeInput<'_>) -> Result<(RunRecord, EvaluationCard)> {
    let timestamp = format_rfc3339_ms(input.now_ms)?;
    let evaluation_card = EvaluationCard {
        schema_version: EVALUATION_CARD_SCHEMA.to_string(),
        provenance: Provenance {
            model: provenance_model(input.metadata),
            model_version: provenance_model_version(input.metadata),
            temperature: provenance_temperature(input.metadata),
            seed_count: 1,
            prompt_hash: combined_hash(
                input
                    .metadata
                    .prompt_tasks
                    .iter()
                    .filter_map(|task| task.prompt_hash.as_deref())
                    .collect(),
            ),
            rubric_hash: combined_hash(
                input
                    .metadata
                    .prompt_tasks
                    .iter()
                    .filter_map(|task| task.rubric_hash.as_deref())
                    .collect(),
            ),
            fixture_refs: declared_fixture_refs(input.metadata.spec_path.as_deref())?,
        },
        cost_usd: input
            .metadata
            .prompt_tasks
            .iter()
            .filter_map(|task| task.cost_usd)
            .sum(),
        timestamp,
    };

    let run_record = RunRecord {
        schema_version: RUN_RECORD_SCHEMA.to_string(),
        run_id: input.run_id.to_string(),
        benchmark_id: input.eval.id.clone(),
        config_id: input.config_id.to_string(),
        runner_kind: input.runner_kind.to_string(),
        output_dir: input.output_dir.to_string(),
        run_report: input.run_report_path.to_string(),
        evidence_path: input.metadata.evidence_path.clone(),
        spec_path: input.metadata.spec_path.clone(),
        artifacts: input.eval.artifacts.clone(),
        score: RunScore {
            metric: input.eval.score.metric.to_string(),
            successes: input.eval.score.successes,
            n: input.eval.score.n,
            point: input.eval.score.point,
            lower: input.eval.score.lower,
            upper: input.eval.score.upper,
            confidence: input.eval.score.confidence,
            method: input.eval.score.method.to_string(),
        },
        evaluation_card: evaluation_card.clone(),
    };
    Ok((run_record, evaluation_card))
}

fn extract_metadata(eval: &EvalReport) -> Result<EvidenceMetadata> {
    let mut metadata = EvidenceMetadata::default();
    for artifact in &eval.artifacts {
        if artifact.ends_with(".json") {
            let path = Path::new(artifact);
            let value = read_json_artifact(path)?;
            if value["schema_version"] == "crucible.prompt_run_evidence.v1" {
                merge_prompt_metadata(&mut metadata, artifact, &value, "prompt")?;
            } else if value["schema_version"] == "crucible.agentic_judge_evidence.v1" {
                merge_prompt_metadata(&mut metadata, artifact, &value, "judge")?;
            } else if value["schema_version"] == "crucible.spec_run_evidence.v1" {
                merge_spec_metadata(&mut metadata, artifact, &value);
            }
        }
    }
    Ok(metadata)
}

/// Shared metadata/task extraction for prompt-shaped evidence: the built-in
/// prompt benchmark runner (`config_prefix = "prompt"`) and the agentic judge
/// runner (`config_prefix = "judge"`, backlog 012). Both write the identical
/// `{runner, provider, model, temperature, system_prompt_hash, tasks[...]}`
/// shape; the prefix only keeps their `config_id` namespaces from colliding
/// when both target the same provider/model.
fn merge_prompt_metadata(
    metadata: &mut EvidenceMetadata,
    artifact: &str,
    value: &Value,
    config_prefix: &str,
) -> Result<()> {
    metadata.runner_kind = value
        .get("runner")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(metadata.runner_kind.take());
    metadata.provider = value
        .get("provider")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(metadata.provider.take());
    metadata.model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(metadata.model.take());
    metadata.spec_path = value
        .get("spec")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(metadata.spec_path.take());
    metadata.temperature = value
        .get("temperature")
        .and_then(Value::as_f64)
        .or(metadata.temperature.take());
    metadata.max_output_units = value
        .get("max_output_units")
        .and_then(Value::as_u64)
        .or(metadata.max_output_units.take());
    metadata.evidence_path = Some(artifact.to_string());

    let provider = metadata.provider.as_deref().unwrap_or("provider");
    let model = metadata.model.as_deref().unwrap_or("model");
    let temperature = metadata
        .temperature
        .map(|value| value.to_string())
        .unwrap_or_else(|| "default".to_string());
    let max_output_units = metadata
        .max_output_units
        .map(|value| value.to_string())
        .unwrap_or_else(|| "default".to_string());
    let system_prompt_hash = value
        .get("system_prompt_hash")
        .and_then(Value::as_str)
        .unwrap_or("prompt");
    metadata.config_id = Some(format!(
        "{config_prefix}:{provider}:{model}:temp={temperature}:max={max_output_units}:prompt={system_prompt_hash}"
    ));

    let tasks = value
        .get("tasks")
        .and_then(Value::as_array)
        .with_context(|| format!("{artifact} is prompt evidence without a tasks array"))?;
    for task in tasks {
        let task_id = task
            .get("task_id")
            .and_then(Value::as_str)
            .with_context(|| format!("{artifact} prompt task is missing task_id"))?;
        let passed = task
            .get("passed")
            .and_then(Value::as_bool)
            .with_context(|| format!("{artifact} prompt task {task_id:?} is missing passed"))?;
        metadata.prompt_tasks.push(PromptTaskInsert {
            task_id: task_id.to_string(),
            class: opt_string(task.get("class")),
            passed,
            latency_ms: opt_u64(task.get("latency_ms")),
            response_id: opt_string(task.get("response_id")),
            requested_model: opt_string(task.get("requested_model")),
            response_model: opt_string(task.get("response_model")),
            prompt_hash: opt_string(task.get("prompt_hash")),
            rubric_hash: opt_string(task.get("rubric_hash")),
            input_units: opt_u64(task.get("prompt_tokens")),
            output_units: opt_u64(task.get("completion_tokens")),
            total_units: opt_u64(task.get("total_tokens")),
            cost_usd: task.get("cost_usd").and_then(Value::as_f64),
            output_text: opt_string(task.get("output")),
            evidence_json: serde_json::to_string(task).context("serializing prompt task row")?,
        });
    }
    Ok(())
}

fn merge_spec_metadata(metadata: &mut EvidenceMetadata, artifact: &str, value: &Value) {
    metadata.runner_kind = value
        .get("runner")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(metadata.runner_kind.take());
    metadata.spec_path = value
        .get("spec")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(metadata.spec_path.take());
    metadata.evidence_path = Some(artifact.to_string());
    if metadata.config_id.is_none() {
        metadata.config_id = value
            .get("corpus")
            .and_then(|corpus| corpus.get("candidate_id"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }

    // Backlog 023: when this run reports pass^k task consistency (present
    // only when every task shares one trial count `k ≥ 2` — see
    // `compute_pass_k`), index each task's pass^k outcome as a paired task
    // row in the same `prompt_task_results` table `compare_configs`'s
    // existing McNemar pairing already reads. That is the entire wire-up: a
    // pass^k comparison across two configs/runs of the same benchmark gets
    // the identical noise-floor verdict `paired_mcnemar` already computes,
    // not a second kernel.
    if value.get("pass_k").is_some_and(|pass_k| !pass_k.is_null()) {
        merge_pass_k_task_rows(metadata, value);
    }
}

/// Reduce a `crucible.spec_run_evidence.v1` run's per-trial `tasks` array to
/// one paired-comparable row per `task_id`: passed iff *every* trial for that
/// task had zero missed defects and zero false positives — the same bar
/// `compute_pass_k` uses to decide whether a task counts toward pass^k.
fn merge_pass_k_task_rows(metadata: &mut EvidenceMetadata, value: &Value) {
    let Some(tasks) = value.get("tasks").and_then(Value::as_array) else {
        return;
    };
    let mut by_task: BTreeMap<&str, bool> = BTreeMap::new();
    for task in tasks {
        let Some(task_id) = task.get("task_id").and_then(Value::as_str) else {
            continue;
        };
        let missed = task.get("missed").and_then(Value::as_u64).unwrap_or(0);
        let false_positives = task
            .get("false_positives")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let trial_passed = missed == 0 && false_positives == 0;
        by_task
            .entry(task_id)
            .and_modify(|passed| *passed = *passed && trial_passed)
            .or_insert(trial_passed);
    }
    for (task_id, passed) in by_task {
        metadata.prompt_tasks.push(PromptTaskInsert {
            task_id: task_id.to_string(),
            class: None,
            passed,
            latency_ms: None,
            response_id: None,
            requested_model: None,
            response_model: None,
            prompt_hash: None,
            rubric_hash: None,
            input_units: None,
            output_units: None,
            total_units: None,
            cost_usd: None,
            output_text: None,
            evidence_json: serde_json::json!({
                "task_id": task_id,
                "pass_k_all_trials_matched": passed,
            })
            .to_string(),
        });
    }
}

fn read_json_artifact(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading run evidence artifact {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {} as JSON", path.display()))
}

fn row_to_stored_run(row: &Row<'_>) -> rusqlite::Result<StoredRun> {
    Ok(StoredRun {
        run_id: row.get(0)?,
        invocation_id: row.get(1)?,
        benchmark_id: row.get(2)?,
        title: row.get(3)?,
        runner_kind: row.get(4)?,
        config_id: row.get(5)?,
        provider: row.get(6)?,
        model: row.get(7)?,
        created_at_unix_ms: row.get(8)?,
        output_dir: row.get(9)?,
        run_report: row.get(10)?,
        evidence_path: row.get(11)?,
        spec_path: row.get(12)?,
        score_metric: row.get(13)?,
        successes: i64_to_u64(row.get(14)?),
        n: i64_to_u64(row.get(15)?),
        point: row.get(16)?,
        lower: row.get(17)?,
        upper: row.get(18)?,
        confidence: row.get(19)?,
        method: row.get(20)?,
    })
}

fn query_artifacts(conn: &Connection, run_id: &str) -> Result<Vec<StoredArtifact>> {
    let mut stmt = conn
        .prepare(
            "SELECT path, kind FROM run_artifacts
             WHERE run_id = ?1
             ORDER BY path",
        )
        .context("preparing artifact query")?;
    let artifacts = stmt
        .query_map(params![run_id], |row| {
            Ok(StoredArtifact {
                path: row.get(0)?,
                kind: row.get(1)?,
            })
        })
        .context("querying artifacts")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("reading artifact rows")?;
    Ok(artifacts)
}

fn query_prompt_tasks(conn: &Connection, run_id: &str) -> Result<Vec<StoredPromptTask>> {
    let mut stmt = conn
        .prepare(
            "SELECT task_id, task_class, passed, latency_ms, response_id, requested_model,
                response_model, prompt_hash, rubric_hash, prompt_tokens,
                completion_tokens, total_tokens, cost_usd, output_text, evidence_json
             FROM prompt_task_results
             WHERE run_id = ?1
             ORDER BY task_id",
        )
        .context("preparing prompt task query")?;
    let tasks = stmt
        .query_map(params![run_id], |row| {
            let evidence_json: String = row.get(14)?;
            Ok(StoredPromptTask {
                task_id: row.get(0)?,
                class: row.get(1)?,
                passed: row.get::<_, i64>(2)? != 0,
                latency_ms: opt_i64_to_u64(row.get(3)?),
                response_id: row.get(4)?,
                requested_model: row.get(5)?,
                response_model: row.get(6)?,
                prompt_hash: row.get(7)?,
                rubric_hash: row.get(8)?,
                input_units: opt_i64_to_u64(row.get(9)?),
                output_units: opt_i64_to_u64(row.get(10)?),
                total_units: opt_i64_to_u64(row.get(11)?),
                cost_usd: row.get(12)?,
                output_text: row.get(13)?,
                evidence_json: serde_json::from_str(&evidence_json)
                    .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?,
            })
        })
        .context("querying prompt tasks")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("reading prompt task rows")?;
    Ok(tasks)
}

fn query_materialization(conn: &Connection, run_id: &str) -> Result<Option<MaterializedRecord>> {
    let materialization = conn
        .query_row(
            "SELECT run_record_json, evaluation_card_json
             FROM run_record_materializations
             WHERE run_id = ?1",
            params![run_id],
            |row| {
                let run_record_json: String = row.get(0)?;
                let evaluation_card_json: String = row.get(1)?;
                Ok((run_record_json, evaluation_card_json))
            },
        )
        .optional()
        .context("querying durable run record")?;
    materialization
        .map(|(run_record_json, evaluation_card_json)| {
            Ok(MaterializedRecord {
                run_record: serde_json::from_str(&run_record_json)
                    .context("parsing stored run record JSON")?,
                evaluation_card: serde_json::from_str(&evaluation_card_json)
                    .context("parsing stored evaluation card JSON")?,
            })
        })
        .transpose()
}

fn latest_for_config(conn: &Connection, benchmark: &str, config: &str) -> Result<StoredRun> {
    conn.query_row(
        "SELECT run_id, invocation_id, benchmark_id, title, runner_kind,
            config_id, provider, model, created_at_unix_ms, output_dir,
            run_report_path, evidence_path, spec_path, score_metric,
            successes, n, point, lower, upper, confidence, score_method
         FROM run_records
         WHERE benchmark_id = ?1 AND (config_id = ?2 OR model = ?2)
         ORDER BY created_at_unix_ms DESC, run_id DESC
         LIMIT 1",
        params![benchmark, config],
        row_to_stored_run,
    )
    .optional()
    .context("querying latest run for config")?
    .with_context(|| format!("no stored run matched config/model {config:?}"))
}

fn artifact_kind(path: &str) -> &'static str {
    if path.ends_with("prompt-run.json") {
        "prompt_run_evidence"
    } else if path.ends_with("task-results.json") {
        "task_results"
    } else if path.ends_with("run-report.json") {
        "run_report"
    } else if path.ends_with(".json") {
        "json"
    } else if path.ends_with(".html") {
        "html"
    } else if path.ends_with(".md") {
        "markdown"
    } else {
        "artifact"
    }
}

fn now_unix_ms() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?;
    i64::try_from(duration.as_millis()).context("current timestamp exceeds i64")
}

fn new_invocation_id(now_ms: i64) -> String {
    let counter = INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("run-{now_ms}-{}-{counter}", std::process::id())
}

fn to_i64<T>(value: T) -> Result<i64>
where
    T: TryInto<i64>,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    value.try_into().context("integer value exceeds i64")
}

fn opt_i64(value: Option<u64>) -> Result<Option<i64>> {
    value.map(to_i64).transpose()
}

fn i64_to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn opt_i64_to_u64(value: Option<i64>) -> Option<u64> {
    value.and_then(|value| u64::try_from(value).ok())
}

fn opt_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

fn opt_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64)
}

fn provenance_model(metadata: &EvidenceMetadata) -> String {
    metadata
        .model
        .clone()
        .or_else(|| {
            metadata
                .prompt_tasks
                .first()
                .and_then(|task| task.requested_model.clone())
        })
        .unwrap_or_else(|| "deterministic".to_string())
}

fn provenance_model_version(metadata: &EvidenceMetadata) -> String {
    let mut versions = metadata
        .prompt_tasks
        .iter()
        .filter_map(|task| task.response_model.as_deref());
    let Some(first) = versions.next() else {
        return String::new();
    };
    if versions.all(|version| version == first) {
        first.to_string()
    } else {
        String::new()
    }
}

fn provenance_temperature(metadata: &EvidenceMetadata) -> Option<f64> {
    if metadata.temperature.is_some() {
        return metadata.temperature;
    }
    if metadata.model.is_none() && metadata.prompt_tasks.is_empty() {
        return Some(0.0);
    }
    None
}

fn combined_hash(values: Vec<&str>) -> String {
    match values.as_slice() {
        [] => String::new(),
        [single] => (*single).to_string(),
        many => stable_hash_bytes(many.iter().map(|value| value.as_bytes())),
    }
}

fn declared_fixture_refs(spec_path: Option<&str>) -> Result<Vec<FixtureRef>> {
    let Some(spec_path) = spec_path else {
        return Ok(Vec::new());
    };
    let Ok(text) = std::fs::read_to_string(spec_path) else {
        eprintln!("warning: could not read eval spec for fixture refs {spec_path}; omitting");
        return Ok(Vec::new());
    };
    let Ok(spec) = serde_json::from_str::<EvalSpec>(&text) else {
        eprintln!("warning: could not parse {spec_path} as EvalSpec for fixture refs; omitting");
        return Ok(Vec::new());
    };
    Ok(spec.fixtures)
}

fn stable_hash_bytes<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn format_rfc3339_ms(unix_ms: i64) -> Result<String> {
    let nanos = i128::from(unix_ms) * 1_000_000;
    let timestamp =
        OffsetDateTime::from_unix_timestamp_nanos(nanos).context("building run timestamp")?;
    timestamp
        .format(&Rfc3339)
        .context("formatting run timestamp")
}

/// Parse a `--since`/`--until` bound: an RFC3339 timestamp
/// (`2026-07-01T00:00:00Z`) or a bare date (`2026-07-01`, taken as UTC
/// midnight), into Unix milliseconds.
pub fn parse_timestamp_bound(raw: &str) -> Result<i64> {
    let timestamp = OffsetDateTime::parse(raw, &Rfc3339).or_else(|_| {
        OffsetDateTime::parse(&format!("{raw}T00:00:00Z"), &Rfc3339)
            .with_context(|| format!("invalid timestamp {raw:?}; expected RFC3339 or YYYY-MM-DD"))
    })?;
    i64::try_from(timestamp.unix_timestamp_nanos() / 1_000_000)
        .context("timestamp exceeds i64 milliseconds")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval_run::{Score, RUN_REPORT_SCHEMA};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crucible-run-store-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn prompt_report(root: &Path, model: &str, success: bool) -> RunReport {
        prompt_report_with_temperature(root, model, success, Some(0))
    }

    fn prompt_report_with_temperature(
        root: &Path,
        model: &str,
        success: bool,
        temperature: Option<u32>,
    ) -> RunReport {
        let out = root.join(model.replace('/', "-"));
        std::fs::create_dir_all(&out).expect("create output dir");
        std::fs::write(
            root.join("prompt-smoke-v0.json"),
            r#"{"schema_version":"crucible.eval_spec.v1","task":"prompt-smoke"}"#,
        )
        .expect("write spec artifact");
        let mut prompt_evidence = serde_json::json!({
            "schema_version": "crucible.prompt_run_evidence.v1",
            "spec_id": "prompt-smoke-v0",
            "spec": root.join("prompt-smoke-v0.json").display().to_string(),
            "runner": "prompt_benchmark",
            "provider": "open_router",
            "model": model,
            "system_prompt_hash": "fnv1a64:test",
            "max_output_units": 8,
            "score": {
                "metric": "prompt_rubric_pass_rate",
                "successes": if success { 1 } else { 0 },
                "n": 1,
                "point": if success { 1.0 } else { 0.0 },
                "lower": 0.0,
                "upper": 1.0,
                "confidence": 0.95,
                "method": "Wilson"
            },
            "totals": {
                "tasks": 1,
                "passed": if success { 1 } else { 0 },
                "failed": if success { 0 } else { 1 }
            },
            "tasks": [{
                "task_id": "exact",
                "class": "format_adherence",
                "prompt_hash": "fnv1a64:prompt",
                "rubric_hash": "fnv1a64:rubric",
                "passed": success,
                "output": if success { "crucible-smoke" } else { "miss" },
                "latency_ms": 42,
                "response_id": "fake-response",
                "requested_model": model,
                "response_model": model,
                "prompt_tokens": 7,
                "completion_tokens": 3,
                "total_tokens": 10,
                "cost_usd": 0.0
            }]
        });
        if let Some(temperature) = temperature {
            prompt_evidence["temperature"] = serde_json::json!(temperature);
        }
        let evidence_path = out.join("prompt-run.json");
        std::fs::write(
            &evidence_path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&prompt_evidence).unwrap()
            ),
        )
        .expect("write prompt evidence");

        RunReport {
            schema_version: RUN_REPORT_SCHEMA,
            output_dir: out.display().to_string(),
            evals: vec![EvalReport {
                id: "prompt-smoke-v0".to_string(),
                title: "Prompt smoke".to_string(),
                score: Score {
                    metric: "prompt_rubric_pass_rate",
                    successes: if success { 1 } else { 0 },
                    n: 1,
                    point: Some(if success { 1.0 } else { 0.0 }),
                    lower: 0.0,
                    upper: 1.0,
                    confidence: 0.95,
                    method: "Wilson",
                },
                artifacts: vec![
                    root.join("prompt-smoke-v0.json").display().to_string(),
                    evidence_path.display().to_string(),
                ],
                notes: Vec::new(),
            }],
        }
    }

    fn agentic_judge_report(root: &Path, model: &str, verdict: bool) -> RunReport {
        let out = root.join(format!("judge-{}", model.replace('/', "-")));
        std::fs::create_dir_all(&out).expect("create output dir");
        std::fs::write(
            root.join("agentic-judge-smoke.json"),
            r#"{"schema_version":"crucible.eval_spec.v1","task":"agentic-judge-smoke"}"#,
        )
        .expect("write spec artifact");
        let judge_evidence = serde_json::json!({
            "schema_version": "crucible.agentic_judge_evidence.v1",
            "spec_id": "agentic-judge-smoke",
            "spec": root.join("agentic-judge-smoke.json").display().to_string(),
            "runner": "agentic_judge",
            "provider": "open_router",
            "model": model,
            "temperature": 0,
            "system_prompt_hash": "fnv1a64:judge-protocol",
            "score": {
                "metric": "judge_pass_rate",
                "successes": if verdict { 1 } else { 0 },
                "n": 1,
                "point": if verdict { 1.0 } else { 0.0 },
                "lower": 0.0,
                "upper": 1.0,
                "confidence": 0.95,
                "method": "Wilson"
            },
            "totals": {
                "tasks": 1,
                "passed": if verdict { 1 } else { 0 },
                "failed": if verdict { 0 } else { 1 }
            },
            "tasks": [{
                "task_id": "real-1",
                "prompt_hash": "fnv1a64:judge-prompt",
                "rubric_hash": "fnv1a64:judge-rubric",
                "expected_pass": serde_json::Value::Null,
                "passed": verdict,
                "output": if verdict { "VERDICT: PASS\ngood" } else { "VERDICT: FAIL\nbad" },
                "latency_ms": 42,
                "response_id": "fake-judge-response",
                "requested_model": model,
                "response_model": model,
                "prompt_tokens": 7,
                "completion_tokens": 3,
                "total_tokens": 10,
                "cost_usd": 0.0
            }]
        });
        let evidence_path = out.join("agentic-judge-run.json");
        std::fs::write(
            &evidence_path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&judge_evidence).unwrap()
            ),
        )
        .expect("write agentic judge evidence");

        RunReport {
            schema_version: RUN_REPORT_SCHEMA,
            output_dir: out.display().to_string(),
            evals: vec![EvalReport {
                id: "agentic-judge-smoke".to_string(),
                title: "Agentic judge smoke".to_string(),
                score: Score {
                    metric: "judge_pass_rate",
                    successes: if verdict { 1 } else { 0 },
                    n: 1,
                    point: Some(if verdict { 1.0 } else { 0.0 }),
                    lower: 0.0,
                    upper: 1.0,
                    confidence: 0.95,
                    method: "Wilson",
                },
                artifacts: vec![
                    root.join("agentic-judge-smoke.json").display().to_string(),
                    evidence_path.display().to_string(),
                ],
                notes: Vec::new(),
            }],
        }
    }

    #[test]
    fn persists_agentic_judge_provenance_under_a_distinct_config_namespace() {
        let root = temp_dir("judge-persist");
        let db = root.join("runs.sqlite");
        let report = agentic_judge_report(&root, "test/judge-model", true);
        persist_report(&db, &report).expect("persist judge report");

        let list = list_runs(
            &db,
            RunListFilter {
                benchmark: Some("agentic-judge-smoke"),
                ..Default::default()
            },
        )
        .expect("list runs");
        assert_eq!(list.runs.len(), 1);
        assert_eq!(list.runs[0].runner_kind, "agentic_judge");
        assert_eq!(list.runs[0].model.as_deref(), Some("test/judge-model"));
        assert!(
            list.runs[0].config_id.starts_with("judge:"),
            "judge runs get a distinct config namespace from prompt runs: {}",
            list.runs[0].config_id
        );

        let detail = show_run(&db, &list.runs[0].run_id).expect("show run");
        assert_eq!(detail.prompt_tasks.len(), 1);
        assert_eq!(detail.prompt_tasks[0].task_id, "real-1");
        let card = detail
            .evaluation_card
            .as_ref()
            .expect("evaluation card is persisted");
        assert_eq!(
            card["provenance"]["model"], "test/judge-model",
            "the judge model is recorded as run provenance"
        );
        assert_eq!(card["provenance"]["prompt_hash"], "fnv1a64:judge-prompt");
        assert_eq!(card["provenance"]["rubric_hash"], "fnv1a64:judge-rubric");
    }

    #[test]
    fn persists_prompt_run_rows_and_artifact_pointers() {
        let root = temp_dir("persist");
        let db = root.join("runs.sqlite");
        let report = prompt_report(&root, "test/model-a", true);
        let receipt = persist_report(&db, &report).expect("persist report");

        assert_eq!(receipt.run_records, 1);
        assert_eq!(receipt.prompt_task_results, 1);

        let list = list_runs(
            &db,
            RunListFilter {
                benchmark: Some("prompt-smoke-v0"),
                ..Default::default()
            },
        )
        .expect("list runs");
        assert_eq!(list.runs.len(), 1);
        assert_eq!(list.runs[0].benchmark_id, "prompt-smoke-v0");
        assert_eq!(list.runs[0].model.as_deref(), Some("test/model-a"));
        assert_eq!(list.runs[0].score_metric, "prompt_rubric_pass_rate");
        assert!(
            list.runs[0].config_id.contains("temp=0") && list.runs[0].config_id.contains("max=8"),
            "prompt config id preserves runner params: {}",
            list.runs[0].config_id
        );

        let detail = show_run(&db, &list.runs[0].run_id).expect("show run");
        assert_eq!(detail.artifacts.len(), 2);
        assert_eq!(detail.prompt_tasks.len(), 1);
        assert_eq!(detail.prompt_tasks[0].task_id, "exact");
        assert_eq!(
            detail.prompt_tasks[0].class.as_deref(),
            Some("format_adherence")
        );
        assert_eq!(detail.prompt_tasks[0].input_units, Some(7));
        assert_eq!(
            detail.prompt_tasks[0].output_text.as_deref(),
            Some("crucible-smoke")
        );
        let card = detail
            .evaluation_card
            .as_ref()
            .expect("evaluation card is persisted");
        assert_eq!(card["schema_version"], "crucible.evaluation_card.v1");
        assert_eq!(card["provenance"]["model"], "test/model-a");
        assert_eq!(card["provenance"]["model_version"], "test/model-a");
        assert_eq!(card["provenance"]["temperature"], 0.0);
        assert_eq!(card["provenance"]["prompt_hash"], "fnv1a64:prompt");
        assert_eq!(card["provenance"]["rubric_hash"], "fnv1a64:rubric");
        assert!(
            card["provenance"].get("fixture_refs").is_none(),
            "fixtures are omitted when the spec declares none: {card}"
        );
        assert_eq!(card["cost_usd"], 0.0);
        assert!(
            card["timestamp"]
                .as_str()
                .expect("timestamp string")
                .ends_with('Z'),
            "timestamp is RFC3339 UTC: {card}"
        );

        let record = detail.run_record.as_ref().expect("run record is persisted");
        assert_eq!(record["schema_version"], "crucible.run_record.v1");
        assert_eq!(record["benchmark_id"], "prompt-smoke-v0");
        assert_eq!(record["score"]["metric"], "prompt_rubric_pass_rate");
        assert_eq!(record["evaluation_card"], *card);
    }

    #[test]
    fn persist_report_reopens_an_existing_populated_db_without_data_loss() {
        // persist_report opens its own Connection per call (open_initialized),
        // so calling it twice against the same path is exactly the "reopen an
        // existing populated ledger" scenario a second `crucible run`
        // invocation hits in practice — not a simulated one.
        let root = temp_dir("reopen");
        let db = root.join("runs.sqlite");

        let first = prompt_report(&root, "test/model-a", true);
        persist_report(&db, &first).expect("persist first report into a fresh db");

        let second = prompt_report(&root, "test/model-b", false);
        persist_report(&db, &second)
            .expect("persist second report into the reopened, already-populated db");

        let list = list_runs(
            &db,
            RunListFilter {
                benchmark: Some("prompt-smoke-v0"),
                ..Default::default()
            },
        )
        .expect("list runs after reopen");
        assert_eq!(
            list.runs.len(),
            2,
            "both runs survive the reopen — init_schema's CREATE TABLE IF NOT \
             EXISTS does not clobber the first run's rows: {:?}",
            list.runs
        );
        let models: std::collections::HashSet<&str> = list
            .runs
            .iter()
            .filter_map(|run| run.model.as_deref())
            .collect();
        assert!(models.contains("test/model-a"), "{models:?}");
        assert!(models.contains("test/model-b"), "{models:?}");

        // Both rows are independently readable, not just listed — a reopen
        // that silently corrupted one run's detail rows while leaving the
        // summary row intact would slip past the count-only assertion above.
        for run in &list.runs {
            let detail = show_run(&db, &run.run_id).expect("show run after reopen");
            assert_eq!(detail.prompt_tasks.len(), 1);
        }
    }

    #[test]
    fn list_runs_respects_limit_and_offset() {
        let root = temp_dir("pagination");
        let db = root.join("runs.sqlite");

        // Five distinct runs under the same benchmark, persisted in order
        // model-0 .. model-4; created_at_unix_ms ties break on run_id DESC
        // (see the ORDER BY in list_runs), so seed a strictly increasing
        // ordinal into the config id via the model slug to make the expected
        // page order unambiguous without depending on wall-clock timing.
        for i in 0..5 {
            let report = prompt_report(&root, &format!("model-{i}"), true);
            persist_report(&db, &report).expect("persist report");
        }

        let unpaged = list_runs(
            &db,
            RunListFilter {
                benchmark: Some("prompt-smoke-v0"),
                ..Default::default()
            },
        )
        .expect("list all runs");
        assert_eq!(
            unpaged.runs.len(),
            5,
            "no limit set means every matching row still comes back, unchanged from before pagination existed"
        );

        let page_one = list_runs(
            &db,
            RunListFilter {
                benchmark: Some("prompt-smoke-v0"),
                limit: Some(2),
                offset: Some(0),
                ..Default::default()
            },
        )
        .expect("list first page");
        assert_eq!(page_one.runs.len(), 2, "limit=2 returns exactly 2 rows");

        let page_two = list_runs(
            &db,
            RunListFilter {
                benchmark: Some("prompt-smoke-v0"),
                limit: Some(2),
                offset: Some(2),
                ..Default::default()
            },
        )
        .expect("list second page");
        assert_eq!(
            page_two.runs.len(),
            2,
            "offset=2, limit=2 returns the next 2 rows"
        );
        assert_ne!(
            page_one.runs[0].run_id, page_two.runs[0].run_id,
            "the second page does not repeat the first page's rows"
        );
        assert_ne!(
            page_one.runs[1].run_id, page_two.runs[0].run_id,
            "the second page does not repeat the first page's rows"
        );

        let page_three = list_runs(
            &db,
            RunListFilter {
                benchmark: Some("prompt-smoke-v0"),
                limit: Some(2),
                offset: Some(4),
                ..Default::default()
            },
        )
        .expect("list third (partial) page");
        assert_eq!(
            page_three.runs.len(),
            1,
            "the last page only has the one remaining row"
        );

        let page_four = list_runs(
            &db,
            RunListFilter {
                benchmark: Some("prompt-smoke-v0"),
                limit: Some(2),
                offset: Some(6),
                ..Default::default()
            },
        )
        .expect("list past the end");
        assert!(
            page_four.runs.is_empty(),
            "an offset past the last row returns no rows, not an error"
        );

        // Every row across the pages accounts for all 5 without duplicates.
        let mut paged_ids: Vec<&str> = page_one
            .runs
            .iter()
            .chain(page_two.runs.iter())
            .chain(page_three.runs.iter())
            .map(|run| run.run_id.as_str())
            .collect();
        paged_ids.sort_unstable();
        let mut unpaged_ids: Vec<&str> =
            unpaged.runs.iter().map(|run| run.run_id.as_str()).collect();
        unpaged_ids.sort_unstable();
        assert_eq!(
            paged_ids, unpaged_ids,
            "paging through with limit=2 covers exactly the same rows as the unpaged list"
        );
    }

    #[test]
    fn omitted_prompt_temperature_stays_absent_in_the_card() {
        let root = temp_dir("no-temperature");
        let db = root.join("runs.sqlite");
        let report = prompt_report_with_temperature(&root, "test/model-a", true, None);
        persist_report(&db, &report).expect("persist report");

        let list = list_runs(
            &db,
            RunListFilter {
                benchmark: Some("prompt-smoke-v0"),
                ..Default::default()
            },
        )
        .expect("list runs");
        let detail = show_run(&db, &list.runs[0].run_id).expect("show run");
        let card = detail
            .evaluation_card
            .as_ref()
            .expect("evaluation card is persisted");
        assert_eq!(card["provenance"]["model"], "test/model-a");
        assert!(
            card["provenance"].get("temperature").is_none(),
            "provider-default temperature must not be rewritten to 0.0: {card}"
        );
    }

    #[test]
    fn missing_fixture_spec_path_does_not_abort_persistence() {
        let root = temp_dir("missing-fixture-spec");
        let db = root.join("runs.sqlite");
        let report = prompt_report(&root, "test/model-a", true);
        let prompt_path = Path::new(&report.evals[0].artifacts[1]);
        let mut evidence: Value = serde_json::from_str(
            &std::fs::read_to_string(prompt_path).expect("read prompt evidence"),
        )
        .expect("prompt evidence is JSON");
        evidence["spec"] = serde_json::json!(root.join("missing-spec.json").display().to_string());
        std::fs::write(
            prompt_path,
            format!("{}\n", serde_json::to_string_pretty(&evidence).unwrap()),
        )
        .expect("rewrite prompt evidence");

        persist_report(&db, &report).expect("missing fixture refs do not abort persistence");
        let list = list_runs(
            &db,
            RunListFilter {
                benchmark: Some("prompt-smoke-v0"),
                ..Default::default()
            },
        )
        .expect("list runs");
        let detail = show_run(&db, &list.runs[0].run_id).expect("show run");
        let card = detail
            .evaluation_card
            .as_ref()
            .expect("evaluation card is persisted");
        assert!(
            card["provenance"].get("fixture_refs").is_none(),
            "unreadable fixture refs are omitted: {card}"
        );
    }

    #[test]
    fn compares_latest_runs_by_model_as_a_paired_mcnemar_delta() {
        // Both fixtures use the fixed task id "exact", so the two runs share a
        // task and the comparison pairs on it instead of falling back.
        let root = temp_dir("compare");
        let db = root.join("runs.sqlite");
        persist_report(&db, &prompt_report(&root, "test/model-a", false)).expect("persist left");
        persist_report(&db, &prompt_report(&root, "test/model-b", true)).expect("persist right");

        let comparison =
            compare_configs(&db, "prompt-smoke-v0", "test/model-a", "test/model-b", 0.05)
                .expect("compare configs");
        assert_eq!(comparison.left.model.as_deref(), Some("test/model-a"));
        assert_eq!(comparison.right.model.as_deref(), Some("test/model-b"));
        assert_eq!(comparison.delta_point, Some(1.0));
        assert_eq!(comparison.comparison_kind, "paired_mcnemar");
        assert_eq!(comparison.common_tasks, 1);
        let paired = comparison.paired.expect("paired outcome present");
        // left failed & right passed on the one shared task: b = 0, c = 1.
        assert_eq!(paired.b, 0);
        assert_eq!(paired.c, 1);
        assert_eq!(
            paired.verdict,
            crucible_core::DeltaVerdict::InsideNoiseFloor,
            "a single discordant pair cannot clear any reasonable noise floor"
        );
        assert_eq!(comparison.class_breakdowns.len(), 1);
        let class = &comparison.class_breakdowns[0];
        assert_eq!(class.class, "format_adherence");
        assert_eq!(class.left_successes, 0);
        assert_eq!(class.left_n, 1);
        assert_eq!(class.right_successes, 1);
        assert_eq!(class.right_n, 1);
        assert!(class.paired.is_some());
    }

    #[test]
    fn compares_prompt_runs_by_class_breakdown() {
        let root = temp_dir("compare-by-class");
        let db = root.join("runs.sqlite");

        let left = prompt_report(&root, "test/model-a", false);
        let right = prompt_report(&root, "test/model-b", true);
        let left_path = Path::new(&left.evals[0].artifacts[1]);
        let right_path = Path::new(&right.evals[0].artifacts[1]);
        for (path, code_passed, logic_passed) in
            [(left_path, false, true), (right_path, true, true)]
        {
            let mut evidence: Value =
                serde_json::from_str(&std::fs::read_to_string(path).expect("read evidence"))
                    .expect("evidence is JSON");
            evidence["tasks"] = serde_json::json!([
                {
                    "task_id": "code-1",
                    "class": "code_output",
                    "prompt_hash": "fnv1a64:code-prompt",
                    "rubric_hash": "fnv1a64:code-rubric",
                    "passed": code_passed,
                    "output": "code",
                    "latency_ms": 1,
                    "requested_model": "test/model",
                    "response_model": "test/model"
                },
                {
                    "task_id": "logic-1",
                    "class": "arithmetic_logic",
                    "prompt_hash": "fnv1a64:logic-prompt",
                    "rubric_hash": "fnv1a64:logic-rubric",
                    "passed": logic_passed,
                    "output": "42",
                    "latency_ms": 1,
                    "requested_model": "test/model",
                    "response_model": "test/model"
                }
            ]);
            std::fs::write(
                path,
                format!("{}\n", serde_json::to_string_pretty(&evidence).unwrap()),
            )
            .expect("rewrite evidence");
        }

        persist_report(&db, &left).expect("persist left");
        persist_report(&db, &right).expect("persist right");
        let comparison =
            compare_configs(&db, "prompt-smoke-v0", "test/model-a", "test/model-b", 0.05)
                .expect("compare configs");

        assert_eq!(comparison.class_breakdowns.len(), 2);
        let by_class: HashMap<&str, &ClassComparison> = comparison
            .class_breakdowns
            .iter()
            .map(|row| (row.class.as_str(), row))
            .collect();
        let code = by_class["code_output"];
        assert_eq!(code.left_successes, 0);
        assert_eq!(code.left_n, 1);
        assert_eq!(code.right_successes, 1);
        assert_eq!(code.right_n, 1);
        assert_eq!(code.delta_point, Some(1.0));
        assert_eq!(code.common_tasks, 1);
        assert!(code.paired.is_some());

        let logic = by_class["arithmetic_logic"];
        assert_eq!(logic.left_successes, 1);
        assert_eq!(logic.right_successes, 1);
        assert_eq!(logic.delta_point, Some(0.0));
    }

    #[test]
    fn compares_latest_runs_without_shared_tasks_falls_back_to_unpaired_delta() {
        let root = temp_dir("compare-unpaired");
        let db = root.join("runs.sqlite");

        let left = prompt_report(&root, "test/model-a", false);
        let left_evidence_path = Path::new(&left.evals[0].artifacts[1]);
        let mut left_evidence: Value = serde_json::from_str(
            &std::fs::read_to_string(left_evidence_path).expect("read left evidence"),
        )
        .expect("left evidence is JSON");
        left_evidence["tasks"][0]["task_id"] = serde_json::json!("left-only");
        std::fs::write(
            left_evidence_path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&left_evidence).unwrap()
            ),
        )
        .expect("rewrite left evidence with a distinct task id");
        persist_report(&db, &left).expect("persist left");
        persist_report(&db, &prompt_report(&root, "test/model-b", true)).expect("persist right");

        let comparison =
            compare_configs(&db, "prompt-smoke-v0", "test/model-a", "test/model-b", 0.05)
                .expect("compare configs");
        assert_eq!(
            comparison.comparison_kind,
            "latest_unpaired_descriptive_delta"
        );
        assert_eq!(comparison.common_tasks, 0);
        assert!(comparison.paired.is_none());
    }

    #[test]
    fn db_write_path_inside_checkout_must_stay_under_runs() {
        let err = validate_db_write_path(Path::new("crucible-runs.sqlite"))
            .expect_err("repo-local DB outside runs is rejected");
        assert!(
            err.to_string().contains("runs/"),
            "error points callers at the gitignored runs tree: {err}"
        );
        let cwd = std::env::current_dir().expect("current dir");
        validate_db_write_path(&cwd.join("tracked.sqlite"))
            .expect_err("absolute repo-local DB outside runs is rejected");
        validate_db_write_path(Path::new("runs/local/crucible-runs.sqlite"))
            .expect("repo-local DB under runs is allowed");
    }

    #[test]
    fn opening_the_run_ledger_sets_a_nonzero_busy_timeout() {
        // Every `open_initialized` call opens its own short-lived Connection
        // (list_runs, show_run, persist_report, compare_configs each open
        // independently), so concurrent readers/writers against the same
        // sqlite file are a real, not theoretical, contention path. Without a
        // busy_timeout pragma, SQLITE_BUSY surfaces immediately instead of
        // rusqlite retrying for a bounded window.
        let root = temp_dir("busy-timeout");
        let db = root.join("runs.sqlite");
        let conn = open_initialized(&db).expect("open a fresh run ledger");
        let busy_timeout_ms: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("read the busy_timeout pragma back");
        assert_eq!(
            busy_timeout_ms, RUN_LEDGER_BUSY_TIMEOUT_MS as i64,
            "run ledger connections must set the explicit busy_timeout, not rely on an implicit default"
        );
    }

    #[test]
    fn parse_timestamp_bound_accepts_rfc3339_and_bare_date() {
        let rfc3339 = parse_timestamp_bound("2026-07-01T00:00:00Z").expect("RFC3339 parses");
        let bare_date = parse_timestamp_bound("2026-07-01").expect("bare date parses");
        assert_eq!(
            rfc3339, bare_date,
            "a bare date is UTC midnight of the same instant as the equivalent RFC3339 timestamp"
        );

        let midday =
            parse_timestamp_bound("2026-07-01T12:30:00Z").expect("RFC3339 with a time parses");
        assert!(
            midday > rfc3339,
            "a later time of day on the same date parses to a later Unix ms value"
        );
    }

    #[test]
    fn parse_timestamp_bound_rejects_an_empty_string() {
        let err = parse_timestamp_bound("").expect_err("an empty string is not a timestamp");
        let message = err.to_string();
        assert!(
            message.contains("invalid timestamp") && message.contains("\"\""),
            "error names the empty value and the field's expected shape: {message}"
        );
    }

    #[test]
    fn parse_timestamp_bound_rejects_garbage() {
        let err =
            parse_timestamp_bound("not-a-date").expect_err("garbage input is not a timestamp");
        let message = err.to_string();
        assert!(
            message.contains("not-a-date") && message.contains("RFC3339"),
            "error names the offending value and the accepted formats: {message}"
        );
    }
}
