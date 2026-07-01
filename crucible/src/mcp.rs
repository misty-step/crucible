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
use crate::spec_run;

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
        "crucible_run" => crucible_run(arguments),
        other => Err(anyhow!("unknown tool: {other}")),
    }
}

#[derive(Debug, Default, Deserialize)]
struct CrucibleRunArgs {
    spec: Option<PathBuf>,
    out: Option<PathBuf>,
    eval: Option<String>,
}

fn crucible_run(arguments: Value) -> Result<Value> {
    let args: CrucibleRunArgs =
        serde_json::from_value(arguments).context("parse crucible_run arguments")?;
    let report = if let Some(spec) = args.spec {
        let eval = args.eval.as_deref().unwrap_or("all");
        if eval != RunEval::All.id() {
            anyhow::bail!("eval selects built-in receipts and cannot be combined with spec");
        }
        spec_run::run(&spec, args.out.as_deref())?
    } else {
        let out = args
            .out
            .as_deref()
            .ok_or_else(|| anyhow!("built-in receipt runs require out"))?;
        eval_run::run(parse_run_eval(args.eval.as_deref())?, out)?
    };

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
            "report": report_json
        }
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

        assert_eq!(names, vec!["crucible_run"]);
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
}
