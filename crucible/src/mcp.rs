//! Minimal Model Context Protocol server for Crucible.
//!
//! Transport is stdio with one JSON-RPC 2.0 message per line. stdout is the
//! protocol channel; diagnostics go to stderr. The server intentionally exposes
//! the shared `crucible run` execution path rather than a second eval runner.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::eval_run::{self, RunEval};
use crate::run_store;
use crate::spec_run;
use crate::validate;

const PROTOCOL_VERSION: &str = "2025-11-25";

pub fn run_stdio() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let line = line.context("read MCP stdin")?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(err) => {
                eprintln!("mcp: invalid JSON: {err}");
                continue;
            }
        };

        let id = message.get("id").cloned();
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let should_shutdown = method == "shutdown";
        let result = dispatch(method, &message);

        if let Some(id) = id {
            let response = match result {
                Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
                Err(err) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32603, "message": err.to_string() }
                }),
            };
            writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
            stdout.flush()?;
        }

        if should_shutdown {
            break;
        }
    }

    Ok(())
}

fn dispatch(method: &str, message: &Value) -> Result<Value> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": message["params"]["protocolVersion"]
                .as_str()
                .unwrap_or(PROTOCOL_VERSION),
            "serverInfo": {
                "name": "crucible",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": { "tools": { "listChanged": false } }
        })),
        "tools/list" => Ok(json!({ "tools": tool_defs() })),
        "tools/call" => call_tool(&message["params"]),
        "ping" | "shutdown" => Ok(json!({})),
        other => Err(anyhow!("method not found: {other}")),
    }
}

fn tool_defs() -> Value {
    json!([
        {
            "name": "crucible_validate",
            "description": "Check whether a declared Crucible EvalSpec is an executable contract: every preflight rule crucible_run enforces (aggregation, uncertainty method/confidence, required grader kind), without needing a runnable corpus. Returns valid/runnable booleans plus named errors and warnings — call this before crucible_run to check a spec is well-formed.",
            "inputSchema": {
                "type": "object",
                "required": ["spec"],
                "properties": {
                    "spec": {
                        "type": "string",
                        "description": "Path to a declared Crucible EvalSpec JSON."
                    }
                }
            }
        },
        {
            "name": "crucible_run",
            "description": "Run a declared Crucible EvalSpec or one/all built-in eval receipts. Returns the scored crucible.run_report.v1 object with Wilson intervals and writes the same run-report.json evidence packet to disk.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "spec": {
                        "type": "string",
                        "description": "Path to a declared Crucible EvalSpec JSON. When present, eval must be omitted or 'all'."
                    },
                    "out": {
                        "type": "string",
                        "description": "Output directory for run-report.json and evidence artifacts. Optional for declared specs; required for built-in receipt evals."
                    },
                    "db": {
                        "type": "string",
                        "description": "SQLite run ledger path. Defaults to runs/local/crucible-runs.sqlite."
                    },
                    "model": {
                        "type": "string",
                        "description": "Override a declared prompt_benchmark spec's configured OpenRouter model slug for this run."
                    },
                    "eval": {
                        "type": "string",
                        "enum": [
                            "all",
                            "code-review-deterministic-floor",
                            "recoverable-adjudication-queue",
                            "harbor-export-acceptance"
                        ],
                        "default": "all",
                        "description": "Built-in receipt eval selector when no spec path is supplied."
                    }
                }
            }
        },
        {
            "name": "crucible_grade",
            "description": "Run the deterministic pre-grader: score a Cerberus review artifact's findings against a Daedalus answer key. Returns matched/disputed/missed counts and a Wilson-scored match rate. The same computation crucible grade --json emits.",
            "inputSchema": {
                "type": "object",
                "required": ["artifact", "key"],
                "properties": {
                    "artifact": {
                        "type": "string",
                        "description": "Path to the Cerberus review artifact JSON."
                    },
                    "key": {
                        "type": "string",
                        "description": "Path to a Daedalus answer key JSON — either the solution/findings.json oracle or the tests/expected.json span scorer key."
                    }
                }
            }
        },
        {
            "name": "crucible_adjudicate",
            "description": "Grade a Cerberus artifact against a Daedalus answer key and build the adjudication queue (disputed findings plus already-applied labels). With apply, mints and includes labels from a JSON array of label decisions. The same computation crucible adjudicate --json emits — use this to drive a headless labeling loop mid-agent-run instead of adjudication-panel --serve.",
            "inputSchema": {
                "type": "object",
                "required": ["artifact", "key"],
                "properties": {
                    "artifact": {
                        "type": "string",
                        "description": "Path to the Cerberus review artifact JSON."
                    },
                    "key": {
                        "type": "string",
                        "description": "Path to a Daedalus answer key JSON — either the solution/findings.json oracle or the tests/expected.json span scorer key."
                    },
                    "apply": {
                        "type": "string",
                        "description": "Path to a JSON array of label decisions to apply to the queue. Each entry names a finding_id present in the queue plus its verdict, disposition, and (optionally) the conditions it was committed under."
                    }
                }
            }
        },
        {
            "name": "crucible_export",
            "description": "Turn a labeled judgment queue (adjudicate --apply output) into the Daedalus key-extension artifacts under out: adjudications.md always, solution/findings.json when key is given, tests/expected.json when expected is given. The same computation crucible export performs; every output is rendered before anything is written, so a bad key/expected fails fast with no half-written tree.",
            "inputSchema": {
                "type": "object",
                "required": ["labels", "out", "arena", "task", "base_version"],
                "properties": {
                    "labels": {
                        "type": "string",
                        "description": "Path to a labeled judgment queue JSON — the adjudicate --apply output."
                    },
                    "out": {
                        "type": "string",
                        "description": "Output directory; adjudications.md (and solution/findings.json, tests/expected.json when requested) are written under it."
                    },
                    "arena": {
                        "type": "string",
                        "description": "Arena id for the log title and Harbor path, e.g. pr-review-v0."
                    },
                    "task": {
                        "type": "string",
                        "description": "Harbor task id the findings were raised against, e.g. py-file-cache."
                    },
                    "base_version": {
                        "type": "string",
                        "description": "Arena version the first ACCEPT bumps from, e.g. 0.2.0."
                    },
                    "date": {
                        "type": "string",
                        "description": "Date to stamp each adjudication with (e.g. 2026-06-29); optional.",
                        "default": ""
                    },
                    "key": {
                        "type": "string",
                        "description": "Original point oracle (solution/findings.json) to extend with the accepted findings. When omitted, no solution/findings.json is written."
                    },
                    "expected": {
                        "type": "string",
                        "description": "Original scorer key (tests/expected.json) to extend with the accepted findings as line-span defects. When omitted, no tests/expected.json is written."
                    }
                }
            }
        },
        {
            "name": "crucible_runs_list",
            "description": "List stored Crucible run records from the SQLite ledger, optionally filtered by benchmark id, config id, model slug, or creation date.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db": {
                        "type": "string",
                        "description": "SQLite run ledger path. Defaults to runs/local/crucible-runs.sqlite."
                    },
                    "benchmark": {
                        "type": "string",
                        "description": "Benchmark id to filter on, such as prompt-smoke-v0."
                    },
                    "config": {
                        "type": "string",
                        "description": "Config id to filter on."
                    },
                    "model": {
                        "type": "string",
                        "description": "Model slug to filter on."
                    },
                    "since": {
                        "type": "string",
                        "description": "Only runs created at or after this RFC3339 timestamp or YYYY-MM-DD date."
                    },
                    "until": {
                        "type": "string",
                        "description": "Only runs created at or before this RFC3339 timestamp or YYYY-MM-DD date."
                    }
                }
            }
        },
        {
            "name": "crucible_runs_show",
            "description": "Show one stored Crucible run record, including artifact pointers and indexed prompt task rows.",
            "inputSchema": {
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "db": {
                        "type": "string",
                        "description": "SQLite run ledger path. Defaults to runs/local/crucible-runs.sqlite."
                    },
                    "run_id": {
                        "type": "string",
                        "description": "Run id returned by crucible_runs_list."
                    }
                }
            }
        },
        {
            "name": "crucible_runs_compare",
            "description": "Compare the latest stored runs for two configs or model slugs under one benchmark. When both runs share prompt task fixtures the result is a paired McNemar outcome; otherwise it is a descriptive latest-run delta that makes no significance claim. When the paired verdict is a statistical signal, set include_findings and/or findings_out to also mint a crucible.findings_journal.v1 record — the same defensible-finding computation crucible runs compare --findings-out performs. Unpaired and inside-noise-floor comparisons always mint zero finding records.",
            "inputSchema": {
                "type": "object",
                "required": ["benchmark", "left", "right"],
                "properties": {
                    "db": {
                        "type": "string",
                        "description": "SQLite run ledger path. Defaults to runs/local/crucible-runs.sqlite."
                    },
                    "benchmark": {
                        "type": "string",
                        "description": "Benchmark id to compare under."
                    },
                    "left": {
                        "type": "string",
                        "description": "Left config id or model slug."
                    },
                    "right": {
                        "type": "string",
                        "description": "Right config id or model slug."
                    },
                    "alpha": {
                        "type": "number",
                        "description": "Significance threshold for the paired McNemar verdict. Defaults to 0.05.",
                        "default": 0.05
                    },
                    "include_findings": {
                        "type": "boolean",
                        "description": "Include a findings_journal object inline in the response (empty findings array unless the paired verdict is a signal). Defaults to false; omitting it leaves the response identical to before this option existed.",
                        "default": false
                    },
                    "findings_out": {
                        "type": "string",
                        "description": "Also write the findings journal JSON to this path, exactly like crucible runs compare --findings-out. Implies include_findings."
                    }
                }
            }
        }
    ])
}

fn call_tool(params: &Value) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("tools/call missing tool name"))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match name {
        "crucible_validate" => crucible_validate(arguments),
        "crucible_run" => crucible_run(arguments),
        "crucible_grade" => crucible_grade(arguments),
        "crucible_adjudicate" => crucible_adjudicate(arguments),
        "crucible_export" => crucible_export(arguments),
        "crucible_runs_list" => crucible_runs_list(arguments),
        "crucible_runs_show" => crucible_runs_show(arguments),
        "crucible_runs_compare" => crucible_runs_compare(arguments),
        other => Err(anyhow!("unknown tool: {other}")),
    }
}

#[derive(Debug, Deserialize)]
struct CrucibleValidateArgs {
    spec: PathBuf,
}

fn crucible_validate(arguments: Value) -> Result<Value> {
    let args: CrucibleValidateArgs =
        serde_json::from_value(arguments).context("parse crucible_validate arguments")?;
    let report = validate::validate(&args.spec)?;
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&report)? }],
        "structuredContent": report
    }))
}

#[derive(Debug, Default, Deserialize)]
struct CrucibleRunArgs {
    spec: Option<PathBuf>,
    out: Option<PathBuf>,
    eval: Option<String>,
    db: Option<PathBuf>,
    model: Option<String>,
}

fn crucible_run(arguments: Value) -> Result<Value> {
    let args: CrucibleRunArgs =
        serde_json::from_value(arguments).context("parse crucible_run arguments")?;
    let report = if let Some(spec) = args.spec {
        let eval = args.eval.as_deref().unwrap_or("all");
        if eval != RunEval::All.id() {
            anyhow::bail!("eval selects built-in receipts and cannot be combined with spec");
        }
        let options = match args.model.as_deref() {
            Some(model) => spec_run::RunOptions::with_prompt_model(model),
            None => spec_run::RunOptions::default(),
        };
        spec_run::run_with_options(&spec, args.out.as_deref(), &options)?
    } else {
        if args.model.is_some() {
            anyhow::bail!("model override can only be used with a declared prompt_benchmark spec");
        }
        let out = args
            .out
            .as_deref()
            .ok_or_else(|| anyhow!("built-in receipt runs require out"))?;
        eval_run::run(parse_run_eval(args.eval.as_deref())?, out)?
    };
    let db_path = args
        .db
        .unwrap_or_else(|| PathBuf::from(run_store::DEFAULT_DB_PATH));
    let stored = run_store::persist_report(&db_path, &report)?;

    let report_json = serde_json::to_value(&report)?;
    let report_text = serde_json::to_string_pretty(&report)?;
    let run_report_path = Path::new(&report.output_dir)
        .join("run-report.json")
        .display()
        .to_string();

    Ok(json!({
        "content": [{ "type": "text", "text": report_text }],
        "structuredContent": {
            "schema_version": report.schema_version,
            "output_dir": report.output_dir,
            "run_report": run_report_path,
            "run_store": stored,
            "report": report_json
        }
    }))
}

#[derive(Debug, Deserialize)]
struct CrucibleGradeArgs {
    artifact: PathBuf,
    key: PathBuf,
}

fn crucible_grade(arguments: Value) -> Result<Value> {
    let args: CrucibleGradeArgs =
        serde_json::from_value(arguments).context("parse crucible_grade arguments")?;
    let report = crate::build_grade_report(&args.artifact, &args.key)?;
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&report)? }],
        "structuredContent": report
    }))
}

#[derive(Debug, Deserialize)]
struct CrucibleAdjudicateArgs {
    artifact: PathBuf,
    key: PathBuf,
    apply: Option<PathBuf>,
}

fn crucible_adjudicate(arguments: Value) -> Result<Value> {
    let args: CrucibleAdjudicateArgs =
        serde_json::from_value(arguments).context("parse crucible_adjudicate arguments")?;
    let queue = crate::build_judgment_queue(&args.artifact, &args.key, args.apply.as_deref())?;
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&queue)? }],
        "structuredContent": queue
    }))
}

#[derive(Debug, Deserialize)]
struct CrucibleExportArgs {
    labels: PathBuf,
    out: PathBuf,
    arena: String,
    task: String,
    base_version: String,
    #[serde(default)]
    date: String,
    key: Option<PathBuf>,
    expected: Option<PathBuf>,
}

fn crucible_export(arguments: Value) -> Result<Value> {
    let args: CrucibleExportArgs =
        serde_json::from_value(arguments).context("parse crucible_export arguments")?;
    let report = crate::build_export(&crate::ExportRequest {
        labels: &args.labels,
        out: &args.out,
        arena: &args.arena,
        task: &args.task,
        base_version: &args.base_version,
        date: &args.date,
        key: args.key.as_deref(),
        expected: args.expected.as_deref(),
    })?;
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&report)? }],
        "structuredContent": report
    }))
}

#[derive(Debug, Default, Deserialize)]
struct RunsListArgs {
    db: Option<PathBuf>,
    benchmark: Option<String>,
    config: Option<String>,
    model: Option<String>,
    since: Option<String>,
    until: Option<String>,
}

fn crucible_runs_list(arguments: Value) -> Result<Value> {
    let args: RunsListArgs =
        serde_json::from_value(arguments).context("parse crucible_runs_list arguments")?;
    let db = args
        .db
        .unwrap_or_else(|| PathBuf::from(run_store::DEFAULT_DB_PATH));
    let since_unix_ms = args
        .since
        .as_deref()
        .map(run_store::parse_timestamp_bound)
        .transpose()?;
    let until_unix_ms = args
        .until
        .as_deref()
        .map(run_store::parse_timestamp_bound)
        .transpose()?;
    let filter = run_store::RunListFilter {
        benchmark: args.benchmark.as_deref(),
        config: args.config.as_deref(),
        model: args.model.as_deref(),
        since_unix_ms,
        until_unix_ms,
    };
    let list = run_store::list_runs(&db, filter)?;
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&list)? }],
        "structuredContent": list
    }))
}

#[derive(Debug, Deserialize)]
struct RunsShowArgs {
    db: Option<PathBuf>,
    run_id: String,
}

fn crucible_runs_show(arguments: Value) -> Result<Value> {
    let args: RunsShowArgs =
        serde_json::from_value(arguments).context("parse crucible_runs_show arguments")?;
    let db = args
        .db
        .unwrap_or_else(|| PathBuf::from(run_store::DEFAULT_DB_PATH));
    let detail = run_store::show_run(&db, &args.run_id)?;
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&detail)? }],
        "structuredContent": detail
    }))
}

#[derive(Debug, Deserialize)]
struct RunsCompareArgs {
    db: Option<PathBuf>,
    benchmark: String,
    left: String,
    right: String,
    #[serde(default = "default_alpha")]
    alpha: f64,
    #[serde(default)]
    include_findings: bool,
    findings_out: Option<PathBuf>,
}

fn default_alpha() -> f64 {
    run_store::DEFAULT_ALPHA
}

/// Compare the latest stored runs for two configs/models. The response's
/// `structuredContent` is the same `ConfigComparison` object emitted before
/// findings journals existed; `include_findings`/`findings_out` are additive
/// fields inserted alongside it, never a replacement for it, so a caller that
/// omits both sees byte-for-byte the same shape as before this option existed.
fn crucible_runs_compare(arguments: Value) -> Result<Value> {
    let args: RunsCompareArgs =
        serde_json::from_value(arguments).context("parse crucible_runs_compare arguments")?;
    let db = args
        .db
        .unwrap_or_else(|| PathBuf::from(run_store::DEFAULT_DB_PATH));
    let comparison =
        run_store::compare_configs(&db, &args.benchmark, &args.left, &args.right, args.alpha)?;

    let mut structured = serde_json::to_value(&comparison).context("serializing comparison")?;
    if args.include_findings || args.findings_out.is_some() {
        let repro_command = crate::runs_compare_repro_command(
            &db,
            &args.benchmark,
            &args.left,
            &args.right,
            args.alpha,
        );
        let journal = match args.findings_out.as_deref() {
            Some(path) => crate::findings_journal::write_journal(
                &comparison,
                args.alpha,
                repro_command,
                path,
            )?,
            None => crate::findings_journal::journal_from_comparison(
                &comparison,
                args.alpha,
                repro_command,
            ),
        };
        let map = structured
            .as_object_mut()
            .expect("ConfigComparison serializes to a JSON object");
        map.insert(
            "findings_journal".to_string(),
            serde_json::to_value(&journal).context("serializing findings journal")?,
        );
        if let Some(path) = args.findings_out.as_deref() {
            map.insert(
                "findings_journal_path".to_string(),
                json!(path.display().to_string()),
            );
        }
    }

    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&structured)? }],
        "structuredContent": structured
    }))
}

fn parse_run_eval(raw: Option<&str>) -> Result<RunEval> {
    match raw.unwrap_or(RunEval::All.id()) {
        "all" => Ok(RunEval::All),
        "code-review-deterministic-floor" => Ok(RunEval::CodeReviewDeterministicFloor),
        "recoverable-adjudication-queue" => Ok(RunEval::RecoverableAdjudicationQueue),
        "harbor-export-acceptance" => Ok(RunEval::HarborExportAcceptance),
        other => Err(anyhow!(
            "unsupported eval {other}; expected one of all, code-review-deterministic-floor, recoverable-adjudication-queue, harbor-export-acceptance"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_exposes_run_as_an_agent_intent() {
        let tools = tool_defs();
        let names = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "crucible_validate",
                "crucible_run",
                "crucible_grade",
                "crucible_adjudicate",
                "crucible_export",
                "crucible_runs_list",
                "crucible_runs_show",
                "crucible_runs_compare"
            ]
        );
    }

    #[test]
    fn eval_parser_accepts_cli_ids() {
        assert_eq!(parse_run_eval(None).unwrap(), RunEval::All);
        assert_eq!(
            parse_run_eval(Some("harbor-export-acceptance")).unwrap(),
            RunEval::HarborExportAcceptance
        );
        assert!(parse_run_eval(Some("missing")).is_err());
    }

    #[test]
    fn runs_compare_omits_findings_journal_by_default() {
        let db = crate::test_fixtures::temp_db("mcp-default");
        crate::test_fixtures::seed_paired_signal(&db);

        let response = crucible_runs_compare(json!({
            "db": db.display().to_string(),
            "benchmark": crate::test_fixtures::BENCHMARK,
            "left": crate::test_fixtures::LEFT_MODEL,
            "right": crate::test_fixtures::RIGHT_MODEL,
        }))
        .expect("crucible_runs_compare succeeds");

        let structured = &response["structuredContent"];
        // The existing comparison object is untouched: same fields as before
        // this card, and no findings_journal key when it was not requested.
        assert_eq!(structured["comparison_kind"], "paired_mcnemar");
        assert_eq!(structured["common_tasks"], 10);
        assert!(
            structured.get("findings_journal").is_none(),
            "findings_journal must be absent unless requested: {structured}"
        );
    }

    #[test]
    fn runs_compare_can_include_a_findings_journal_inline_for_a_paired_signal() {
        let db = crate::test_fixtures::temp_db("mcp-include-signal");
        crate::test_fixtures::seed_paired_signal(&db);

        let response = crucible_runs_compare(json!({
            "db": db.display().to_string(),
            "benchmark": crate::test_fixtures::BENCHMARK,
            "left": crate::test_fixtures::LEFT_MODEL,
            "right": crate::test_fixtures::RIGHT_MODEL,
            "include_findings": true,
        }))
        .expect("crucible_runs_compare succeeds");

        let structured = &response["structuredContent"];
        assert_eq!(
            structured["comparison_kind"], "paired_mcnemar",
            "the existing comparison object is preserved alongside the addition"
        );
        let journal = &structured["findings_journal"];
        assert_eq!(journal["schema_version"], "crucible.findings_journal.v1");
        let findings = journal["findings"].as_array().expect("findings array");
        assert_eq!(
            findings.len(),
            1,
            "a paired signal must mint exactly one finding record: {journal}"
        );
        assert_eq!(findings[0]["paired"]["verdict"], "signal");
    }

    #[test]
    fn runs_compare_can_write_a_findings_journal_to_disk() {
        let db = crate::test_fixtures::temp_db("mcp-write-signal");
        crate::test_fixtures::seed_paired_signal(&db);
        let out_dir = db.parent().expect("db has a parent dir").to_path_buf();
        let findings_out = out_dir.join("findings.json");

        let response = crucible_runs_compare(json!({
            "db": db.display().to_string(),
            "benchmark": crate::test_fixtures::BENCHMARK,
            "left": crate::test_fixtures::LEFT_MODEL,
            "right": crate::test_fixtures::RIGHT_MODEL,
            "findings_out": findings_out.display().to_string(),
        }))
        .expect("crucible_runs_compare succeeds");

        let structured = &response["structuredContent"];
        assert_eq!(
            structured["findings_journal_path"],
            findings_out.display().to_string()
        );
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(&findings_out).expect("read written findings journal"),
        )
        .expect("written findings journal is JSON");
        assert_eq!(written["findings"].as_array().expect("findings").len(), 1);
        // The same journal is also returned inline, not just written (allow
        // for last-ULP float round-trip drift through the written JSON text).
        assert_eq!(
            structured["findings_journal"]["schema_version"],
            written["schema_version"]
        );
        assert_eq!(
            structured["findings_journal"]["findings"][0]["id"],
            written["findings"][0]["id"]
        );
        assert_eq!(
            structured["findings_journal"]["findings"][0]["paired"]["verdict"],
            written["findings"][0]["paired"]["verdict"]
        );
    }

    #[test]
    fn runs_compare_findings_journal_is_empty_inside_the_noise_floor() {
        let db = crate::test_fixtures::temp_db("mcp-noise-floor");
        crate::test_fixtures::seed_paired_inside_noise_floor(&db);

        let response = crucible_runs_compare(json!({
            "db": db.display().to_string(),
            "benchmark": crate::test_fixtures::BENCHMARK,
            "left": crate::test_fixtures::LEFT_MODEL,
            "right": crate::test_fixtures::RIGHT_MODEL,
            "include_findings": true,
        }))
        .expect("crucible_runs_compare succeeds");

        let structured = &response["structuredContent"];
        assert_eq!(structured["comparison_kind"], "paired_mcnemar");
        assert_eq!(
            structured["findings_journal"]["findings"]
                .as_array()
                .expect("findings array")
                .len(),
            0,
            "an inside-noise-floor paired comparison must mint no finding records: {structured}"
        );
    }
}
