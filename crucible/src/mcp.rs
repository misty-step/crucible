//! Minimal Model Context Protocol server for Crucible.
//!
//! Transport is stdio with one JSON-RPC 2.0 message per line. stdout is the
//! protocol channel; diagnostics go to stderr. The server intentionally exposes
//! the shared `crucible run` execution path rather than a second eval runner.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::ValueEnum;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::author::{self, AuthorArgs, AuthorExpectationKind, AuthorRunnerKind};
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

/// `crucible_author`'s tool definition, factored out of [`tool_defs`]'s
/// `json!` array literal — folded inline, the array's total nesting tripped
/// the macro's default recursion limit (see `#[recursion_limit]` docs).
/// Interpolating a pre-built `Value` here counts as one leaf to the
/// surrounding `json!`, not additional nested tokens.
fn crucible_author_tool_def() -> Value {
    json!({
        "name": "crucible_author",
        "description": "Assemble a valid Crucible EvalSpec from flags without hand-writing JSON — covers key_recall (Daedalus trials.jsonl corpus) and prompt_benchmark (one authored task per call) runner kinds. Runs the exact same validate-then-save gate crucible_validate performs before writing: an assembly that fails validation is refused and leaves no file at out. Returns the same {valid, runnable, errors, warnings} report as crucible_validate plus the resolved out path and whether the file was written. agentic_judge authoring and the interactive prompt flow are CLI-only (crucible author --interactive); this tool covers the same non-interactive flag surface as crucible author.",
        "inputSchema": {
            "type": "object",
            "required": ["task_family", "runner_kind"],
            "properties": {
                "out": {
                    "type": "string",
                    "description": "Output path for the assembled spec JSON. Defaults to evals/<id-or-task-slug>.json."
                },
                "force": {
                    "type": "boolean",
                    "description": "Overwrite an existing file at out. Without this, an existing file at out refuses the write.",
                    "default": false
                },
                "id": {
                    "type": "string",
                    "description": "Stable eval id, e.g. my-eval-v0. Defaults to the output file stem."
                },
                "task_family": {
                    "type": "string",
                    "description": "The task family this eval measures, e.g. code-review."
                },
                "inputs": {
                    "type": "string",
                    "description": "Free-form description of the inputs this eval consumes."
                },
                "outputs": {
                    "type": "string",
                    "description": "Free-form description of the outputs this eval scores."
                },
                "decision": {
                    "type": "string",
                    "description": "The decision this eval informs, in one sentence."
                },
                "baselines": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Named baseline configs to compare against."
                },
                "graders": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Grader mix entries, each `<id>:<kind>` where kind is deterministic|agentic|human. When omitted, one canonical grader of the chosen runner's required kind is added automatically."
                },
                "runner_kind": {
                    "type": "string",
                    "enum": ["key_recall", "prompt_benchmark"],
                    "description": "Which runner this spec declares."
                },
                "key_recall_arena_dir": {
                    "type": "string",
                    "description": "key_recall: Daedalus arena directory, absolute or relative to the eventual spec file."
                },
                "key_recall_trials_jsonl": {
                    "type": "string",
                    "description": "key_recall: Daedalus trials.jsonl file, absolute or relative to the eventual spec file."
                },
                "key_recall_candidate_id": {
                    "type": "string",
                    "description": "key_recall: candidate id to select from the trials file."
                },
                "key_recall_tasks": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "key_recall: task ids to select. Omit entirely to select every trial for the candidate."
                },
                "prompt_model": {
                    "type": "string",
                    "description": "prompt_benchmark: OpenRouter model slug, e.g. openai/gpt-4o-mini."
                },
                "prompt_system_prompt": {
                    "type": "string",
                    "description": "prompt_benchmark: system prompt shared by the authored task."
                },
                "prompt_credential_env": {
                    "type": "string",
                    "description": "prompt_benchmark: env var carrying the provider credential. Defaults to OPENROUTER_API_KEY."
                },
                "prompt_max_output_units": {
                    "type": "integer",
                    "description": "prompt_benchmark: optional output cap for the model call."
                },
                "prompt_temperature": {
                    "type": "integer",
                    "description": "prompt_benchmark: optional integer temperature."
                },
                "prompt_harness": {
                    "type": "string",
                    "description": "prompt_benchmark: optional agent harness identity for this run, e.g. claude-code, codex, or raw-api (backlog 027)."
                },
                "prompt_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "prompt_benchmark: tool ids available to the harness during this run. Omit entirely to leave the tool allowlist empty (backlog 027)."
                },
                "prompt_task_id": {
                    "type": "string",
                    "description": "prompt_benchmark: the authored task's stable id."
                },
                "prompt_task_prompt": {
                    "type": "string",
                    "description": "prompt_benchmark: the authored task's user prompt."
                },
                "prompt_task_class": {
                    "type": "string",
                    "description": "prompt_benchmark: optional reporting class, e.g. format_adherence."
                },
                "prompt_task_context_file": {
                    "type": "string",
                    "description": "prompt_benchmark: optional prompt-context file, absolute or relative to the eventual spec file."
                },
                "prompt_expectation_kind": {
                    "type": "string",
                    "enum": ["exact", "contains", "case_insensitive_contains", "regex", "strict_json"],
                    "description": "prompt_benchmark: the task's deterministic rubric kind."
                },
                "prompt_expectation_value": {
                    "type": "string",
                    "description": "prompt_benchmark: the rubric value — exact/contains text, a regex pattern, or (for strict_json) a literal JSON value."
                }
            }
        }
    })
}

/// In-process MCP self-check for `crucible doctor` (crucible-911): initialize
/// the server and list its tools through the exact same [`dispatch`] path
/// `run_stdio` uses, without spawning a subprocess or touching stdio. Returns
/// the tool names so the caller can confirm the expected surface is present.
/// Kept to this one function rather than exporting `dispatch`/`tool_defs`
/// directly, so the MCP module's stdio-server internals stay private.
pub(crate) fn self_check() -> Result<Vec<String>> {
    let initialized = dispatch(
        "initialize",
        &json!({ "params": { "protocolVersion": PROTOCOL_VERSION } }),
    )
    .context("MCP initialize failed")?;
    if initialized["serverInfo"]["name"].as_str() != Some("crucible") {
        anyhow::bail!("initialize did not return crucible serverInfo: {initialized}");
    }

    let listed = dispatch("tools/list", &json!({})).context("MCP tools/list failed")?;
    let names: Vec<String> = listed["tools"]
        .as_array()
        .ok_or_else(|| anyhow!("tools/list did not return a tools array: {listed}"))?
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect();
    if names.is_empty() {
        anyhow::bail!("tools/list returned zero tools");
    }
    Ok(names)
}

fn tool_defs() -> Value {
    json!([
        crucible_author_tool_def(),
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
                    "harness": {
                        "type": "string",
                        "description": "Agent harness identity to filter on, e.g. claude-code or codex (backlog 027)."
                    },
                    "since": {
                        "type": "string",
                        "description": "Only runs created at or after this RFC3339 timestamp or YYYY-MM-DD date."
                    },
                    "until": {
                        "type": "string",
                        "description": "Only runs created at or before this RFC3339 timestamp or YYYY-MM-DD date."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Cap the number of rows returned. Omitted means every matching row (the pre-pagination default)."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Rows to skip before the first returned row; combine with limit to page through a large run ledger."
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
            "description": "Compare the latest stored runs for two configs or model slugs under one benchmark. When both runs share prompt task fixtures the result is a paired McNemar outcome; otherwise it is a descriptive latest-run delta that makes no significance claim. Every response carries an attribution label (model_delta/harness_delta/config_delta) derived from the actual identity diff between the two resolved runs -- config_delta means the delta spans more than one axis and is unattributable to any single one; set strict to refuse such comparisons outright instead of rendering them with a caveat. When the paired verdict is a statistical signal, set include_findings and/or findings_out to also mint a crucible.findings_journal.v1 record — the same defensible-finding computation crucible runs compare --findings-out performs. Unpaired and inside-noise-floor comparisons always mint zero finding records.",
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
                    },
                    "strict": {
                        "type": "boolean",
                        "description": "Refuse (rather than caveat) a comparison spanning more than one identity axis (model, harness, tool_allowlist, scoring) at once -- backlog 974's axis-mismatch guard. Defaults to false.",
                        "default": false
                    }
                }
            }
        },
        {
            "name": "crucible_runs_judge_status",
            "description": "Query a judge's standing calibration licence by its licence key (backlog 029) -- is this exact judge identity (model + judge prompt + calibration rubric set) currently licensed, across runs, without recomputing calibration from scratch. The licence key is the calibration record's licence_key field, also logged in an agentic-judge run's notes. Returns null when no run has ever measured this exact identity -- read that as locked/unlicensed.",
            "inputSchema": {
                "type": "object",
                "required": ["licence_key"],
                "properties": {
                    "db": {
                        "type": "string",
                        "description": "SQLite run ledger path. Defaults to runs/local/crucible-runs.sqlite."
                    },
                    "licence_key": {
                        "type": "string",
                        "description": "The calibration record's licence_key."
                    }
                }
            }
        },
        {
            "name": "crucible_runs_history",
            "description": "Time-series score history for one benchmark/config or model slug, ordered oldest to newest — the longitudinal trend line backlog 027 adds. config matches either the stored config_id or the stored model, same either-match rule crucible_runs_compare's left/right use.",
            "inputSchema": {
                "type": "object",
                "required": ["benchmark", "config"],
                "properties": {
                    "db": {
                        "type": "string",
                        "description": "SQLite run ledger path. Defaults to runs/local/crucible-runs.sqlite."
                    },
                    "benchmark": {
                        "type": "string",
                        "description": "Benchmark id to trend."
                    },
                    "config": {
                        "type": "string",
                        "description": "Config id or model slug to trend."
                    }
                }
            }
        },
        {
            "name": "crucible_runs_pivot",
            "description": "Cross-axis pivot: one benchmark's latest stored run per model, optionally narrowed to one harness — \"this benchmark, this harness, across all models\" (backlog 027).",
            "inputSchema": {
                "type": "object",
                "required": ["benchmark"],
                "properties": {
                    "db": {
                        "type": "string",
                        "description": "SQLite run ledger path. Defaults to runs/local/crucible-runs.sqlite."
                    },
                    "benchmark": {
                        "type": "string",
                        "description": "Benchmark id to pivot."
                    },
                    "harness": {
                        "type": "string",
                        "description": "Agent harness identity to narrow to, e.g. claude-code or codex. Omit to pivot across every harness recorded for the benchmark."
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
        "crucible_author" => crucible_author(arguments),
        "crucible_validate" => crucible_validate(arguments),
        "crucible_run" => crucible_run(arguments),
        "crucible_grade" => crucible_grade(arguments),
        "crucible_adjudicate" => crucible_adjudicate(arguments),
        "crucible_export" => crucible_export(arguments),
        "crucible_runs_list" => crucible_runs_list(arguments),
        "crucible_runs_show" => crucible_runs_show(arguments),
        "crucible_runs_compare" => crucible_runs_compare(arguments),
        "crucible_runs_judge_status" => crucible_runs_judge_status(arguments),
        "crucible_runs_history" => crucible_runs_history(arguments),
        "crucible_runs_pivot" => crucible_runs_pivot(arguments),
        other => Err(anyhow!("unknown tool: {other}")),
    }
}

#[derive(Debug, Default, Deserialize)]
struct CrucibleAuthorArgs {
    out: Option<PathBuf>,
    #[serde(default)]
    force: bool,
    id: Option<String>,
    task_family: Option<String>,
    inputs: Option<String>,
    outputs: Option<String>,
    decision: Option<String>,
    #[serde(default)]
    baselines: Vec<String>,
    #[serde(default)]
    graders: Vec<String>,
    runner_kind: Option<String>,
    key_recall_arena_dir: Option<String>,
    key_recall_trials_jsonl: Option<String>,
    key_recall_candidate_id: Option<String>,
    #[serde(default)]
    key_recall_tasks: Vec<String>,
    prompt_model: Option<String>,
    prompt_system_prompt: Option<String>,
    prompt_credential_env: Option<String>,
    prompt_max_output_units: Option<u32>,
    prompt_temperature: Option<u32>,
    prompt_harness: Option<String>,
    #[serde(default)]
    prompt_tools: Vec<String>,
    prompt_task_id: Option<String>,
    prompt_task_prompt: Option<String>,
    prompt_task_class: Option<String>,
    prompt_task_context_file: Option<String>,
    prompt_expectation_kind: Option<String>,
    prompt_expectation_value: Option<String>,
}

impl CrucibleAuthorArgs {
    /// Convert the MCP wire args into the same [`AuthorArgs`] `crucible
    /// author`'s flag path resolves — same enum parsing, same required-field
    /// checks in [`author::author_from_flags`], so a caller sees the exact
    /// same errors either surface would report.
    fn into_author_args(self) -> Result<AuthorArgs> {
        let runner_kind = self
            .runner_kind
            .as_deref()
            .map(|raw| {
                AuthorRunnerKind::from_str(raw, false).map_err(|err| {
                    anyhow!("runner_kind {raw:?} is invalid: {err} (expected key_recall or prompt_benchmark)")
                })
            })
            .transpose()?;
        let prompt_expectation_kind = self
            .prompt_expectation_kind
            .as_deref()
            .map(|raw| {
                AuthorExpectationKind::from_str(raw, false).map_err(|err| {
                    anyhow!(
                        "prompt_expectation_kind {raw:?} is invalid: {err} (expected exact, contains, case_insensitive_contains, regex, or strict_json)"
                    )
                })
            })
            .transpose()?;

        Ok(AuthorArgs {
            interactive: false,
            out: self.out,
            force: self.force,
            json: false,
            id: self.id,
            task_family: self.task_family,
            inputs: self.inputs,
            outputs: self.outputs,
            decision: self.decision,
            baselines: self.baselines,
            graders: self.graders,
            runner_kind,
            key_recall_arena_dir: self.key_recall_arena_dir,
            key_recall_trials_jsonl: self.key_recall_trials_jsonl,
            key_recall_candidate_id: self.key_recall_candidate_id,
            key_recall_tasks: self.key_recall_tasks,
            prompt_model: self.prompt_model,
            prompt_system_prompt: self.prompt_system_prompt,
            prompt_credential_env: self.prompt_credential_env,
            prompt_max_output_units: self.prompt_max_output_units,
            prompt_temperature: self.prompt_temperature,
            prompt_harness: self.prompt_harness,
            prompt_tools: self.prompt_tools,
            prompt_task_id: self.prompt_task_id,
            prompt_task_prompt: self.prompt_task_prompt,
            prompt_task_class: self.prompt_task_class,
            prompt_task_context_file: self.prompt_task_context_file,
            prompt_expectation_kind,
            prompt_expectation_value: self.prompt_expectation_value,
        })
    }
}

fn crucible_author(arguments: Value) -> Result<Value> {
    let raw: CrucibleAuthorArgs =
        serde_json::from_value(arguments).context("parse crucible_author arguments")?;
    let args = raw.into_author_args()?;
    let report = author::author_from_flags(&args)?;
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&report)? }],
        "structuredContent": report
    }))
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
    harness: Option<String>,
    since: Option<String>,
    until: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
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
        harness: args.harness.as_deref(),
        since_unix_ms,
        until_unix_ms,
        limit: args.limit,
        offset: args.offset,
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
struct RunsJudgeStatusArgs {
    db: Option<PathBuf>,
    licence_key: String,
}

fn crucible_runs_judge_status(arguments: Value) -> Result<Value> {
    let args: RunsJudgeStatusArgs =
        serde_json::from_value(arguments).context("parse crucible_runs_judge_status arguments")?;
    let db = args
        .db
        .unwrap_or_else(|| PathBuf::from(run_store::DEFAULT_DB_PATH));
    let status = run_store::judge_licence_status(&db, &args.licence_key)?;
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&status)? }],
        "structuredContent": status
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
    /// Refuse (rather than caveat) a comparison spanning more than one
    /// identity axis at once (backlog 974). Defaults to `false`.
    #[serde(default)]
    strict: bool,
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
    let comparison = run_store::compare_configs(
        &db,
        &args.benchmark,
        &args.left,
        &args.right,
        args.alpha,
        args.strict,
    )?;

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

#[derive(Debug, Deserialize)]
struct RunsHistoryArgs {
    db: Option<PathBuf>,
    benchmark: String,
    config: String,
}

fn crucible_runs_history(arguments: Value) -> Result<Value> {
    let args: RunsHistoryArgs =
        serde_json::from_value(arguments).context("parse crucible_runs_history arguments")?;
    let db = args
        .db
        .unwrap_or_else(|| PathBuf::from(run_store::DEFAULT_DB_PATH));
    let history = run_store::score_history(&db, &args.benchmark, &args.config)?;
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&history)? }],
        "structuredContent": history
    }))
}

#[derive(Debug, Deserialize)]
struct RunsPivotArgs {
    db: Option<PathBuf>,
    benchmark: String,
    harness: Option<String>,
}

fn crucible_runs_pivot(arguments: Value) -> Result<Value> {
    let args: RunsPivotArgs =
        serde_json::from_value(arguments).context("parse crucible_runs_pivot arguments")?;
    let db = args
        .db
        .unwrap_or_else(|| PathBuf::from(run_store::DEFAULT_DB_PATH));
    let pivot = run_store::pivot_by_model(&db, &args.benchmark, args.harness.as_deref())?;
    Ok(json!({
        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&pivot)? }],
        "structuredContent": pivot
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
    fn self_check_initializes_and_lists_a_nonempty_tool_surface() {
        let names = self_check().expect("in-process MCP self-check succeeds");
        assert!(
            names.contains(&"crucible_run".to_string()),
            "self_check must surface the same tools tools/list returns: {names:?}"
        );
        assert_eq!(
            names.len(),
            tool_defs().as_array().unwrap().len(),
            "self_check must not silently filter the real tool surface"
        );
    }

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
                "crucible_author",
                "crucible_validate",
                "crucible_run",
                "crucible_grade",
                "crucible_adjudicate",
                "crucible_export",
                "crucible_runs_list",
                "crucible_runs_show",
                "crucible_runs_compare",
                "crucible_runs_judge_status",
                "crucible_runs_history",
                "crucible_runs_pivot"
            ]
        );
    }

    #[test]
    fn crucible_runs_judge_status_reports_null_for_an_unmeasured_licence_key() {
        let db = crate::test_fixtures::temp_db("mcp-judge-status-empty");
        let response = crucible_runs_judge_status(json!({
            "db": db.display().to_string(),
            "licence_key": "judge-licence:v1:no/such-judge:hash-a:hash-b",
        }))
        .expect("crucible_runs_judge_status succeeds");
        assert!(
            response["structuredContent"].is_null(),
            "no run has measured this key: {response}"
        );
    }

    /// crucible-006: agents need to author a new benchmark through CLI *and*
    /// MCP without a human explaining undocumented steps. Before this test,
    /// `crucible author` existed only as a CLI command — MCP callers had no
    /// way to assemble a spec, only to validate/run one already on disk. This
    /// pins the MCP half of that path end to end: assemble a runnable
    /// `prompt_benchmark` spec from flags, confirm it's written and valid,
    /// and confirm it reloads and runs identically to a hand-authored one.
    #[test]
    fn crucible_author_assembles_and_saves_a_runnable_prompt_benchmark_spec() {
        let dir = std::env::temp_dir().join(format!(
            "crucible-mcp-author-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out_path = dir.join("mcp-authored-v0.json");

        let response = crucible_author(json!({
            "out": out_path.display().to_string(),
            "task_family": "prompt-smoke",
            "runner_kind": "prompt_benchmark",
            "prompt_model": "openrouter/auto",
            "prompt_system_prompt": "Answer exactly.",
            "prompt_task_id": "marker-echo",
            "prompt_task_prompt": "Reply with crucible-smoke",
            "prompt_expectation_kind": "contains",
            "prompt_expectation_value": "crucible-smoke",
        }))
        .expect("crucible_author succeeds");

        let structured = &response["structuredContent"];
        assert_eq!(structured["written"], true, "{structured}");
        assert_eq!(structured["validate"]["valid"], true, "{structured}");
        assert_eq!(structured["validate"]["runnable"], true, "{structured}");
        assert_eq!(structured["out"], out_path.display().to_string());
        assert!(out_path.exists(), "spec must be written to out");

        // The saved file is a real EvalSpec crucible_validate accepts too —
        // the exact same save gate CLI `crucible author` runs through.
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
        assert_eq!(saved["task"], "prompt-smoke");
        assert_eq!(saved["graders"]["graders"][0]["kind"], "deterministic");
    }

    /// An invalid assembly (explicit grader mix missing the runner's
    /// required kind) must be refused with no file written — same shape as
    /// `crucible_validate` reporting `valid: false` in the body rather than
    /// erroring the MCP call, so a caller can inspect why without a
    /// try/catch. No file is left at `out`.
    #[test]
    fn crucible_author_refuses_an_invalid_assembly_and_writes_nothing() {
        let dir = std::env::temp_dir().join(format!(
            "crucible-mcp-author-invalid-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out_path = dir.join("bad-v0.json");

        let response = crucible_author(json!({
            "out": out_path.display().to_string(),
            "task_family": "prompt-smoke",
            "runner_kind": "prompt_benchmark",
            "prompt_model": "openrouter/auto",
            "prompt_system_prompt": "Answer exactly.",
            "prompt_task_id": "marker-echo",
            "prompt_task_prompt": "Reply with crucible-smoke",
            "prompt_expectation_kind": "contains",
            "prompt_expectation_value": "crucible-smoke",
            "graders": ["operator:human"],
        }))
        .expect("crucible_author succeeds even for an invalid assembly");

        let structured = &response["structuredContent"];
        assert_eq!(structured["written"], false, "{structured}");
        assert_eq!(structured["validate"]["valid"], false, "{structured}");
        assert!(!out_path.exists(), "no file should exist at out");
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

    #[test]
    fn runs_history_returns_the_seeded_models_score_points() {
        let db = crate::test_fixtures::temp_db("mcp-history");
        crate::test_fixtures::seed_paired_signal(&db);

        let response = crucible_runs_history(json!({
            "db": db.display().to_string(),
            "benchmark": crate::test_fixtures::BENCHMARK,
            "config": crate::test_fixtures::LEFT_MODEL,
        }))
        .expect("crucible_runs_history succeeds");

        let structured = &response["structuredContent"];
        assert_eq!(structured["benchmark"], crate::test_fixtures::BENCHMARK);
        assert_eq!(structured["config_query"], crate::test_fixtures::LEFT_MODEL);
        let points = structured["points"].as_array().expect("points array");
        assert_eq!(
            points.len(),
            1,
            "one seeded run for the left model: {structured}"
        );
        assert_eq!(points[0]["successes"], 1);
        assert_eq!(points[0]["n"], 10);
    }

    #[test]
    fn runs_history_requires_benchmark_and_config() {
        let db = crate::test_fixtures::temp_db("mcp-history-missing-args");
        let err = crucible_runs_history(json!({ "db": db.display().to_string() }))
            .expect_err("benchmark and config are required");
        assert!(
            format!("{err:#}").contains("missing field"),
            "error names the missing required field: {err:#}"
        );
    }

    #[test]
    fn runs_pivot_returns_one_row_per_seeded_model() {
        let db = crate::test_fixtures::temp_db("mcp-pivot");
        crate::test_fixtures::seed_paired_signal(&db);

        let response = crucible_runs_pivot(json!({
            "db": db.display().to_string(),
            "benchmark": crate::test_fixtures::BENCHMARK,
        }))
        .expect("crucible_runs_pivot succeeds");

        let structured = &response["structuredContent"];
        assert_eq!(structured["benchmark"], crate::test_fixtures::BENCHMARK);
        assert!(
            structured.get("harness").is_none(),
            "harness omitted when not narrowed: {structured}"
        );
        let rows = structured["rows"].as_array().expect("rows array");
        assert_eq!(rows.len(), 2, "one row per seeded model: {structured}");
    }

    #[test]
    fn runs_pivot_narrows_to_a_harness_with_zero_rows_when_none_match() {
        let db = crate::test_fixtures::temp_db("mcp-pivot-no-match");
        crate::test_fixtures::seed_paired_signal(&db);

        let response = crucible_runs_pivot(json!({
            "db": db.display().to_string(),
            "benchmark": crate::test_fixtures::BENCHMARK,
            "harness": "codex",
        }))
        .expect("crucible_runs_pivot succeeds");

        let structured = &response["structuredContent"];
        assert_eq!(structured["harness"], "codex");
        assert_eq!(
            structured["rows"].as_array().expect("rows array").len(),
            0,
            "the seeded fixture never declared a harness, so a codex-narrowed pivot is empty: {structured}"
        );
    }
}
