//! Test-only fixtures shared by the MCP and serve unit tests (see
//! `src/mcp.rs`, `src/serve.rs`).
//!
//! Both faces need a real, persisted paired comparison — one that clears the
//! noise floor (a defensible finding) and one that doesn't — without a live
//! model call. `prompt_benchmark` evidence is data (a `prompt-run.json`
//! artifact `persist_report` indexes into `run_records`/prompt task rows), so
//! it can be hand-built exactly like `run_store`'s own private test helpers do
//! for the same reason. This module exists so that shape is not duplicated
//! twice across mcp.rs and serve.rs.

use std::path::{Path, PathBuf};

use crate::eval_run::{EvalReport, RunReport, Score, RUN_REPORT_SCHEMA};
use crate::run_store;

pub(crate) const BENCHMARK: &str = "findings-fixture-v0";
pub(crate) const LEFT_MODEL: &str = "test/model-a";
pub(crate) const RIGHT_MODEL: &str = "test/model-b";

/// A fresh scratch SQLite path under the system temp dir.
pub(crate) fn temp_db(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "crucible-findings-fixture-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join("runs.sqlite")
}

/// Persist `LEFT_MODEL`/`RIGHT_MODEL` runs sharing 10 prompt tasks with a
/// 1-vs-9 discordant split. `PairedComparison::mcnemar(1, 9)` clears the
/// default `alpha = 0.05` noise floor (p ~= 0.021 per the doctest on
/// `PairedComparison::verdict`), so `compare_configs` returns
/// `DeltaVerdict::Signal` and a findings journal mints exactly one record.
pub(crate) fn seed_paired_signal(db: &Path) {
    seed(db, |i| i == 0, |i| i != 0);
}

/// Persist `LEFT_MODEL`/`RIGHT_MODEL` runs sharing 10 prompt tasks with a
/// 1-vs-1 discordant split — inside the noise floor at any conventional
/// alpha, so a findings journal must mint zero records.
pub(crate) fn seed_paired_inside_noise_floor(db: &Path) {
    seed(db, |i| i == 0, |i| i == 1);
}

fn seed(db: &Path, left_pass: impl Fn(usize) -> bool, right_pass: impl Fn(usize) -> bool) {
    let root = db.parent().expect("db has a parent dir").to_path_buf();
    persist_prompt_run(&root, db, LEFT_MODEL, &left_pass);
    persist_prompt_run(&root, db, RIGHT_MODEL, &right_pass);
}

fn persist_prompt_run(root: &Path, db: &Path, model: &str, passed: &impl Fn(usize) -> bool) {
    let out = root.join(model.replace('/', "-"));
    std::fs::create_dir_all(&out).expect("create output dir");
    let spec_path = root.join(format!("{BENCHMARK}.json"));
    std::fs::write(
        &spec_path,
        r#"{"schema_version":"crucible.eval_spec.v1","task":"findings-fixture"}"#,
    )
    .expect("write spec artifact");

    let tasks: Vec<serde_json::Value> = (0..10)
        .map(|i| {
            let ok = passed(i);
            serde_json::json!({
                "task_id": format!("t{i}"),
                "class": "format_adherence",
                "prompt_hash": format!("fnv1a64:prompt-{i}"),
                "rubric_hash": format!("fnv1a64:rubric-{i}"),
                "passed": ok,
                "tracked_results": [{ "id": "style", "passed": !ok }],
                "output": if ok { "match" } else { "miss" },
                "latency_ms": 1,
                "requested_model": model,
                "response_model": model
            })
        })
        .collect();
    let successes = tasks.iter().filter(|task| task["passed"] == true).count() as u64;
    let n = tasks.len() as u64;
    let point = successes as f64 / n as f64;

    let evidence = serde_json::json!({
        "schema_version": "crucible.prompt_run_evidence.v1",
        "spec_id": BENCHMARK,
        "spec": spec_path.display().to_string(),
        "runner": "prompt_benchmark",
        "provider": "open_router",
        "model": model,
        "system_prompt_hash": "fnv1a64:test",
        "max_output_units": 8,
        "score": {
            "metric": "prompt_rubric_pass_rate",
            "successes": successes,
            "n": n,
            "point": point,
            "lower": 0.0,
            "upper": 1.0,
            "confidence": 0.95,
            "method": "Wilson"
        },
        "totals": { "tasks": n, "passed": successes, "failed": n - successes },
        "tasks": tasks
    });
    let evidence_path = out.join("prompt-run.json");
    std::fs::write(
        &evidence_path,
        format!("{}\n", serde_json::to_string_pretty(&evidence).unwrap()),
    )
    .expect("write prompt evidence");

    let report = RunReport {
        schema_version: RUN_REPORT_SCHEMA,
        output_dir: out.display().to_string(),
        evals: vec![EvalReport {
            id: BENCHMARK.to_string(),
            title: "Findings fixture".to_string(),
            score: Score {
                metric: "prompt_rubric_pass_rate",
                successes,
                n,
                point: Some(point),
                lower: 0.0,
                upper: 1.0,
                confidence: 0.95,
                method: "Wilson",
            },
            artifacts: vec![
                spec_path.display().to_string(),
                evidence_path.display().to_string(),
            ],
            notes: Vec::new(),
        }],
    };
    run_store::persist_report(db, &report).expect("persist prompt report");
}
