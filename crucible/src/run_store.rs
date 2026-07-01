//! SQLite run ledger for Crucible-owned benchmark executions.
//!
//! The ledger is deliberately boring: one invocation row, one row per eval
//! result, artifact pointers, and runner-specific task rows where Crucible knows
//! how to index them. Full JSON copies stay with each row so future
//! `RunRecord`/`EvaluationCard` materialization can migrate forward without
//! reparsing chat or relying on a loose artifact still existing.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use crucible_core::{
    EvalSpec, EvaluationCard, FixtureRef, Provenance, RunRecord, RunScore, EVALUATION_CARD_SCHEMA,
    RUN_RECORD_SCHEMA,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::eval_run::{EvalReport, RunReport};

/// Default local run ledger path. The whole `runs/` tree is gitignored because
/// real eval runs may contain proprietary diffs and raw model output.
pub const DEFAULT_DB_PATH: &str = "runs/local/crucible-runs.sqlite";

const RUN_STORE_SCHEMA: &str = "crucible.run_store.v1";
static INVOCATION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
pub struct PersistedReport {
    pub schema_version: &'static str,
    pub db: String,
    pub invocation_id: String,
    pub output_dir: String,
    pub run_report: String,
    pub run_records: usize,
    pub prompt_task_results: usize,
}

#[derive(Debug, Serialize)]
pub struct RunList {
    pub schema_version: &'static str,
    pub db: String,
    pub benchmark: Option<String>,
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
    pub comparison_kind: &'static str,
    pub note: &'static str,
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
    prompt_tasks: Vec<PromptTaskInsert>,
}

#[derive(Debug, Clone)]
struct PromptTaskInsert {
    task_id: String,
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
                    run_id, task_id, passed, latency_ms, response_id,
                    requested_model, response_model, prompt_hash, rubric_hash,
                    prompt_tokens, completion_tokens, total_tokens, cost_usd,
                    output_text, evidence_json
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    ?14, ?15
                )",
                params![
                    run_id,
                    task.task_id,
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

pub fn list_runs(db_path: &Path, benchmark: Option<&str>) -> Result<RunList> {
    let conn = open_initialized(db_path)?;
    let mut stmt = conn
        .prepare(
            "SELECT run_id, invocation_id, benchmark_id, title, runner_kind,
                config_id, provider, model, created_at_unix_ms, output_dir,
                run_report_path, evidence_path, spec_path, score_metric,
                successes, n, point, lower, upper, confidence, score_method
             FROM run_records
             WHERE (?1 IS NULL OR benchmark_id = ?1)
             ORDER BY created_at_unix_ms DESC, run_id DESC",
        )
        .context("preparing run list query")?;
    let rows = stmt
        .query_map(params![benchmark], row_to_stored_run)
        .context("querying run list")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("reading run list rows")?;

    Ok(RunList {
        schema_version: RUN_STORE_SCHEMA,
        db: db_path.display().to_string(),
        benchmark: benchmark.map(str::to_string),
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

pub fn compare_configs(
    db_path: &Path,
    benchmark: &str,
    left: &str,
    right: &str,
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

    Ok(ConfigComparison {
        schema_version: RUN_STORE_SCHEMA,
        db: db_path.display().to_string(),
        benchmark: benchmark.to_string(),
        left_query: left.to_string(),
        right_query: right.to_string(),
        left: left_run,
        right: right_run,
        delta_point,
        comparison_kind: "latest_unpaired_descriptive_delta",
        note: "This compares the latest matching run per config/model and does not assert statistical significance.",
    })
}

fn open_initialized(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating run database directory {}", parent.display()))?;
    }
    let conn = Connection::open(db_path)
        .with_context(|| format!("opening run database {}", db_path.display()))?;
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
    .context("initializing run-store schema")
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
            temperature: input.metadata.temperature.unwrap_or(0.0),
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
                merge_prompt_metadata(&mut metadata, artifact, &value)?;
            } else if value["schema_version"] == "crucible.spec_run_evidence.v1" {
                merge_spec_metadata(&mut metadata, artifact, &value);
            }
        }
    }
    Ok(metadata)
}

fn merge_prompt_metadata(
    metadata: &mut EvidenceMetadata,
    artifact: &str,
    value: &Value,
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
    metadata.evidence_path = Some(artifact.to_string());

    let provider = metadata.provider.as_deref().unwrap_or("provider");
    let model = metadata.model.as_deref().unwrap_or("model");
    let system_prompt_hash = value
        .get("system_prompt_hash")
        .and_then(Value::as_str)
        .unwrap_or("prompt");
    metadata.config_id = Some(format!("prompt:{provider}:{model}:{system_prompt_hash}"));

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
            "SELECT task_id, passed, latency_ms, response_id, requested_model,
                response_model, prompt_hash, rubric_hash, prompt_tokens,
                completion_tokens, total_tokens, cost_usd, output_text, evidence_json
             FROM prompt_task_results
             WHERE run_id = ?1
             ORDER BY task_id",
        )
        .context("preparing prompt task query")?;
    let tasks = stmt
        .query_map(params![run_id], |row| {
            let evidence_json: String = row.get(13)?;
            Ok(StoredPromptTask {
                task_id: row.get(0)?,
                passed: row.get::<_, i64>(1)? != 0,
                latency_ms: opt_i64_to_u64(row.get(2)?),
                response_id: row.get(3)?,
                requested_model: row.get(4)?,
                response_model: row.get(5)?,
                prompt_hash: row.get(6)?,
                rubric_hash: row.get(7)?,
                input_units: opt_i64_to_u64(row.get(8)?),
                output_units: opt_i64_to_u64(row.get(9)?),
                total_units: opt_i64_to_u64(row.get(10)?),
                cost_usd: row.get(11)?,
                output_text: row.get(12)?,
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
    let text = std::fs::read_to_string(spec_path)
        .with_context(|| format!("reading eval spec for fixture refs {spec_path}"))?;
    let spec: EvalSpec = serde_json::from_str(&text)
        .with_context(|| format!("parsing {spec_path} as EvalSpec for fixture refs"))?;
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
        let out = root.join(model.replace('/', "-"));
        std::fs::create_dir_all(&out).expect("create output dir");
        std::fs::write(
            root.join("prompt-smoke-v0.json"),
            r#"{"schema_version":"crucible.eval_spec.v1","task":"prompt-smoke"}"#,
        )
        .expect("write spec artifact");
        let prompt_evidence = serde_json::json!({
            "schema_version": "crucible.prompt_run_evidence.v1",
            "spec_id": "prompt-smoke-v0",
            "spec": root.join("prompt-smoke-v0.json").display().to_string(),
            "runner": "prompt_benchmark",
            "provider": "open_router",
            "model": model,
            "temperature": 0,
            "system_prompt_hash": "fnv1a64:test",
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

    #[test]
    fn persists_prompt_run_rows_and_artifact_pointers() {
        let root = temp_dir("persist");
        let db = root.join("runs.sqlite");
        let report = prompt_report(&root, "test/model-a", true);
        let receipt = persist_report(&db, &report).expect("persist report");

        assert_eq!(receipt.run_records, 1);
        assert_eq!(receipt.prompt_task_results, 1);

        let list = list_runs(&db, Some("prompt-smoke-v0")).expect("list runs");
        assert_eq!(list.runs.len(), 1);
        assert_eq!(list.runs[0].benchmark_id, "prompt-smoke-v0");
        assert_eq!(list.runs[0].model.as_deref(), Some("test/model-a"));
        assert_eq!(list.runs[0].score_metric, "prompt_rubric_pass_rate");

        let detail = show_run(&db, &list.runs[0].run_id).expect("show run");
        assert_eq!(detail.artifacts.len(), 2);
        assert_eq!(detail.prompt_tasks.len(), 1);
        assert_eq!(detail.prompt_tasks[0].task_id, "exact");
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
    fn compares_latest_runs_by_model_without_claiming_significance() {
        let root = temp_dir("compare");
        let db = root.join("runs.sqlite");
        persist_report(&db, &prompt_report(&root, "test/model-a", false)).expect("persist left");
        persist_report(&db, &prompt_report(&root, "test/model-b", true)).expect("persist right");

        let comparison = compare_configs(&db, "prompt-smoke-v0", "test/model-a", "test/model-b")
            .expect("compare configs");
        assert_eq!(comparison.left.model.as_deref(), Some("test/model-a"));
        assert_eq!(comparison.right.model.as_deref(), Some("test/model-b"));
        assert_eq!(comparison.delta_point, Some(1.0));
        assert_eq!(
            comparison.comparison_kind,
            "latest_unpaired_descriptive_delta"
        );
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
}
