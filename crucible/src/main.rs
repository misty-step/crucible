//! Crucible CLI — evaluate a Cerberus review run against a Daedalus answer key,
//! then queue what the deterministic floor cannot resolve for adjudication.
//!
//! Eight subcommands over Crucible's current eval surface:
//!
//! - `crucible adapt <artifact.json> [--json]` projects every Cerberus finding
//!   onto a Daedalus answer-key row and prints the rows. This is an inspection
//!   view of the adapter, faithful to its **total, order-preserving** contract:
//!   every finding yields one row, even an unanchored one. (No `schema_valid`
//!   filtering here — `adapt` shows the raw projection; `grade` is where the
//!   pre-grader's validity filter applies.)
//! - `crucible grade --artifact <a.json> --key <key.json> [--json]` runs the
//!   deterministic pre-grader — drop schema-invalid findings, project, dedup the
//!   key, then [`grade()`] — and reports matched / disputed / missed counts plus a
//!   Wilson 95% interval on the match rate `matched / (matched + missed)` (recall
//!   over the key); `--key` accepts either Daedalus key shape — the
//!   `solution/findings.json` oracle or the `tests/expected.json` span scorer
//!   key. It also reports `dropped_invalid` (findings the schema-valid
//!   filter removed before grading) and `recoverable_misses` (missed key rows a
//!   disputed finding agrees with on location but not category), so the recall
//!   reads as a category-strict pre-adjudication floor, not a final rate. When
//!   there are no key rows (`n == 0`), the match-rate `point` is `null`, never a
//!   misleading `0.0` — "no data" is not "0%".
//! - `crucible adjudicate --artifact <a.json> --key <key.json> [--apply <labels.json>] [--json]`
//!   grades, then builds the ordered [`JudgmentQueue`] of disputed findings a
//!   judge must rule on (recoverable misses first). With `--apply`, it reads a
//!   JSON array of [`Label`] decisions, validates each against the queue, re-mints
//!   them through [`apply_label`], and emits the labeled judgment artifact — the
//!   headless half of the phone adjudication loop (epic 005).
//! - `crucible export --labels <queue.json> --out <dir> --arena <id> --task <name>
//!   --base-version <X.Y.Z> [--date <date>] [--key <findings.json>]
//!   [--expected <expected.json>]` takes the labeled judgment queue and writes the
//!   Daedalus key-extension artifacts: an `adjudications.md` human log
//!   (ACCEPT→key+version bump / OUT-OF-SCOPE, derived from each label's
//!   verdict+disposition), the extended `solution/findings.json` oracle (`--key`),
//!   and the extended `tests/expected.json` scorer key (`--expected`) that
//!   `daedalus-score` reads — the file an accepted finding must land in to
//!   re-score as a true positive. The write side of the flywheel (epic 002.5).
//! - `crucible dashboard [--arenas <DIR>] [--runs <DIR>] --out <DIR>` ingests the
//!   real Daedalus arenas and runs into a [`Dataset`], computes the
//!   [`Leaderboard`], and writes a self-contained, phone-first `index.html` plus
//!   the full `data.json` model under `<out>`. The read side made viewable: it
//!   recomputes no statistic, only renders the measured ranking — every number
//!   tracing to a run and pinned to its arena version.
//! - `crucible run [<spec.json>] [--out <DIR>] [--eval <ID>] [--json]` either
//!   executes a declared [`EvalSpec`](crucible_core::EvalSpec) runner when a spec
//!   path is supplied (key-recall or prompt benchmark), or runs the three
//!   built-in committed receipt checks when no spec is supplied. Every score
//!   carries a Wilson interval. The run is persisted into Crucible's SQLite run
//!   ledger unless `--db` points at a different ledger. Prompt benchmark specs
//!   can fan out across selected models with `--models a,b,c`; each model is a
//!   normal persisted run with its own config identity.
//! - `crucible runs list|show|compare` queries the SQLite run ledger by
//!   benchmark, run id, or latest config/model pair.
//! - `crucible serve` exposes the same spec validation and run ledger surfaces
//!   as a local-first browser application.
//! - `crucible adjudication-panel --queue <queue.json> --out <DIR>` renders an
//!   existing `crucible.judgment_queue.v1` artifact into a static phone-first
//!   `index.html` panel plus the copied `queue.json` model.
//! - `crucible mcp` serves the shared `crucible run` path over stdio MCP as the
//!   `crucible_run` tool, so agents and Threshold can invoke the same declared
//!   spec runner and get the same Wilson-scored run report.
//! - `crucible author [--interactive] --out <PATH> ...` assembles a valid
//!   [`EvalSpec`](crucible_core::EvalSpec) from flags (cold-agent/scriptable
//!   path) or a guided stdin/stdout prompt flow, runs it through the same
//!   validation `crucible validate` performs, and only writes the spec file
//!   when it is valid (backlog/Powder crucible-942 — the brainstorm/design/
//!   define lifecycle stage previously had no guided surface at all).
//! - `crucible import promptfoo <CONFIG> [--out <PATH>] [--model <SLUG>]
//!   [--force] [--json]` (crucible-026) projects an externally-authored
//!   Promptfoo-style YAML eval config into a valid
//!   [`EvalSpec`](crucible_core::EvalSpec) — the missing "import" direction
//!   `VISION.md` names alongside export — through the same validate-then-save
//!   gate `crucible author` uses. Every source test case is accounted for:
//!   it either becomes a runnable `prompt_benchmark` task or is named in the
//!   printed report as skipped, with why (an unsupported assertion type, more
//!   than one assertion, an unresolved `$ref`/`{{var}}`, ...) — never
//!   silently dropped. See [`crucible_core::import`] for the projection.
//! - `crucible doctor [--json]` (crucible-911) is the push-button,
//!   verified-live onboarding check: it proves the CLI runs, the MCP server
//!   initializes and lists tools, `crucible serve` binds a port and answers
//!   `/api/specs`, and the SQLite run ledger can be created under `runs/` —
//!   then separately reports whether `OPENROUTER_API_KEY` is configured
//!   (`Warn`, never `Fail`, when absent; the value is never printed). See
//!   `doctor.rs` for the exact checks.
//! - `crucible publish --run <RUN_ID> [--db <PATH>] --out <DIR>`
//!   (crucible-publish-packet) is the ONLY door between the private run
//!   ledger and a public benchmark site: a read-only export of one stored
//!   `prompt_benchmark` run (the only publishable runner kind in v1) as a
//!   self-contained `crucible.bench_packet.v1` JSON file. It refuses — never
//!   emits a partial packet — on an unknown run id, any other runner kind, a
//!   missing/unreadable evidence file, an evidence benchmark/model that
//!   disagrees with the run record, or an evidence/spec task-id mismatch. A
//!   missing spec file is tolerated (spec-derived fields go `null`, noted on
//!   stderr) since the evidence alone already carries every ledger-owned
//!   fact. See `publish.rs`.
//!
//! `--json` emits a stable serde object (`adapt`/`grade`/`adjudicate`); the
//! default is a human-readable table. `dashboard` instead writes files under
//! `--out` and prints a short receipt.
//!
//! **Exit codes** are stable so Cerberus/Daedalus can branch headlessly:
//! `0` success, `1` a load/parse failure (a bad artifact, key, or labels file),
//! `2` a usage error (bad arguments, surfaced by clap). `--help`/`--version` exit
//! `0`. `crucible doctor` is the one exception to "verdict lives only in the
//! JSON body": since its whole purpose is a scriptable readiness gate, it
//! exits `1` when any check is broken (not merely a missing optional
//! credential), after printing the full report either way.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use crucible_core::judgment::reconcile_labels;
use crucible_core::{
    adjudications_from_queue, apply_label, build_queue, dedup, extended_expected_key, extended_key,
    findings_from_artifact, grade, proportion, recoverable_misses, render_adjudications_md,
    schema_valid, to_key_findings, wilson_interval, AnswerKey, ArenaVersion, Dataset, Defect,
    ExpectedKey, ExportContext, GradeResult, JudgmentItem, JudgmentQueue, KeyFinding, Label,
    LabelConditions, Leaderboard, SkipReason,
};
use serde::Serialize;
use tracing_subscriber::prelude::*;

mod adjudication_panel;
mod adjudication_server;
mod author;
mod canary;
mod dashboard_html;
mod doctor;
mod eval_run;
mod findings_journal;
mod harbor_import;
mod import;
mod mcp;
mod publish;
mod run_fanout;
mod run_matrix;
mod run_prompt_variants;
mod run_store;
mod serve;
mod spec_run;
mod spec_save;
#[cfg(test)]
mod test_fixtures;
mod validate;

#[cfg(test)]
static LIVE_SOCKET_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
fn live_socket_test_guard() -> std::sync::MutexGuard<'static, ()> {
    LIVE_SOCKET_TEST_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
fn bind_loopback_listener_for_test(context: &str) -> Option<std::net::TcpListener> {
    match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => Some(listener),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping live socket test {context}: loopback bind refused by OS: {err}");
            None
        }
        Err(err) => panic!("{context}: bind ephemeral loopback port: {err}"),
    }
}

/// Standard-normal quantile for a two-sided 95% interval.
const Z_95: f64 = 1.96;
/// The confidence level [`Z_95`] corresponds to, surfaced in reports.
const CONFIDENCE: f64 = 0.95;
/// Max width of the rendered description column before truncation.
const DESC_WIDTH: usize = 56;
/// Exit code for a load or parse failure — a bad artifact, key, or labels file.
/// Usage errors (`2`) are emitted by clap; success is `0`.
const EXIT_LOAD_ERROR: u8 = 1;
/// Stable schema id for the `grade --json` object ([`GradeReport`]), so a headless
/// Cerberus/Daedalus parser can pin the shape — the same stability guarantee
/// `adjudicate` already gives through its judgment-queue schema.
const GRADE_REPORT_SCHEMA: &str = "crucible.grade_report.v1";
/// Stable schema id for the `adapt --json` object ([`AdaptReport`]).
const ADAPT_REPORT_SCHEMA: &str = "crucible.adapt_report.v1";
/// Stable schema id for the `export`/MCP `crucible_export` report
/// ([`ExportReport`]).
const EXPORT_REPORT_SCHEMA: &str = "crucible.export_report.v1";

/// Score a Cerberus review run against a Daedalus answer key.
#[derive(Debug, Parser)]
#[command(name = "crucible", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Project a Cerberus artifact's findings onto Daedalus answer-key rows.
    Adapt {
        /// Path to the Cerberus review artifact JSON.
        #[arg(value_name = "ARTIFACT")]
        artifact: PathBuf,
        /// Emit a stable JSON object instead of a human-readable table.
        #[arg(long)]
        json: bool,
    },
    /// Grade a Cerberus artifact against a Daedalus answer key.
    Grade {
        /// Path to the Cerberus review artifact JSON.
        #[arg(long, value_name = "PATH")]
        artifact: PathBuf,
        /// Path to a Daedalus answer key JSON — either the `solution/findings.json`
        /// oracle or the `tests/expected.json` span scorer key.
        #[arg(long, value_name = "PATH")]
        key: PathBuf,
        /// Emit a stable JSON object instead of a human-readable table.
        #[arg(long)]
        json: bool,
    },
    /// Build an adjudication queue from a grade and optionally apply labels.
    Adjudicate {
        /// Path to the Cerberus review artifact JSON.
        #[arg(long, value_name = "PATH")]
        artifact: PathBuf,
        /// Path to a Daedalus answer key JSON — either the `solution/findings.json`
        /// oracle or the `tests/expected.json` span scorer key.
        #[arg(long, value_name = "PATH")]
        key: PathBuf,
        /// Path to a JSON array of label decisions to apply to the queue. Each
        /// entry names a `finding_id` present in the queue plus its verdict,
        /// disposition, and (optionally) the conditions it was committed under.
        #[arg(long, value_name = "PATH")]
        apply: Option<PathBuf>,
        /// Emit the stable judgment-queue object instead of a readable table.
        #[arg(long)]
        json: bool,
    },
    /// Export a labeled adjudication queue as a Daedalus `adjudications.md`
    /// log, plus the extended `solution/findings.json` oracle (`--key`) and the
    /// extended `tests/expected.json` scorer key (`--expected`).
    Export {
        /// Path to a labeled judgment queue JSON — the `adjudicate --apply`
        /// output: the disputed items plus the labels a judge committed.
        #[arg(long, value_name = "PATH")]
        labels: PathBuf,
        /// Output directory; `adjudications.md` (and `solution/findings.json`
        /// when `--key` is given) are written under it.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Arena id for the log title and Harbor path, e.g. `pr-review-v0`.
        #[arg(long, value_name = "ID")]
        arena: String,
        /// Harbor task id the findings were raised against, e.g. `py-file-cache`.
        #[arg(long, value_name = "NAME")]
        task: String,
        /// Arena version the first ACCEPT bumps from, e.g. `0.2.0`.
        #[arg(long, value_name = "X.Y.Z")]
        base_version: String,
        /// Date to stamp each adjudication with (e.g. `2026-06-29`); optional.
        #[arg(long, value_name = "DATE", default_value = "")]
        date: String,
        /// Original point oracle (`solution/findings.json`) to extend with the
        /// accepted findings. When omitted, no `solution/findings.json` is written.
        #[arg(long, value_name = "PATH")]
        key: Option<PathBuf>,
        /// Original scorer key (`tests/expected.json`) to extend with the accepted
        /// findings as line-span defects — the file `daedalus-score` reads, where
        /// an accepted finding must land to re-score as a true positive. When
        /// omitted, no `tests/expected.json` is written.
        #[arg(long, value_name = "PATH")]
        expected: Option<PathBuf>,
    },
    /// Render the real Daedalus arenas + runs into a self-contained, phone-first
    /// HTML eval dashboard (plus the full `data.json` model) under `--out`.
    Dashboard {
        /// Arenas tree (the answer keys) to read; defaults to the local Daedalus
        /// checkout.
        #[arg(long, value_name = "DIR", default_value = "../daedalus/arenas")]
        arenas: PathBuf,
        /// Runs tree (the trials) to read; defaults to the local Daedalus checkout.
        #[arg(long, value_name = "DIR", default_value = "../daedalus/runs")]
        runs: PathBuf,
        /// Output directory; `index.html` and `data.json` are written under it
        /// (created if absent). Point it at a scratch/gitignored path.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Run a declared eval spec, or one/all built-in eval receipts, and write
    /// evidence under `--out`. Each binary score carries a Wilson interval.
    Run {
        /// Path to a declared Crucible EvalSpec JSON. When present, `run`
        /// executes the spec's runner instead of the built-in receipt selector.
        #[arg(value_name = "SPEC")]
        spec: Option<PathBuf>,
        /// Which built-in eval to run. Defaults to all three concrete receipts.
        #[arg(long, value_enum, default_value_t = eval_run::RunEval::All)]
        eval: eval_run::RunEval,
        /// Output directory for `run-report.json` and per-eval evidence packets.
        /// Declared specs default to `runs/local/<spec-id>` when omitted.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Emit the stable run report JSON to stdout in addition to writing it.
        #[arg(long)]
        json: bool,
        /// Exit non-zero after the run if any tracked prompt check failed.
        /// Scores and persisted records still use gate expectations only.
        #[arg(long)]
        strict_tracked: bool,
        /// Override a prompt_benchmark spec's configured model slug for this
        /// run. This keeps one authored benchmark comparable across selected
        /// OpenRouter models without committing model-specific spec copies.
        #[arg(long, value_name = "SLUG")]
        model: Option<String>,
        /// Run a prompt_benchmark spec once per comma-separated model slug.
        /// Each model writes under a model-specific child directory and persists
        /// as its own run row with its own config identity.
        #[arg(long, value_name = "SLUG,SLUG")]
        models: Option<String>,
        /// Run the spec once in each named operating environment declaration
        /// (a `crucible.environment.v1` JSON file). Repeatable: pass `--env`
        /// two or more times. Each environment overrides the spec's
        /// model-invocation axes (model, provider, temperature, harness,
        /// tool_allowlist, ...), writes under an `env-<id>` child directory,
        /// persists as its own run row with its own config identity, and the
        /// first environment is compared against every later one. This is the
        /// "run eval X in env A vs env B" workbench loop.
        #[arg(long = "env", value_name = "ENV_JSON")]
        envs: Vec<PathBuf>,
        /// Run the prompt-benchmark spec once per selected named system-prompt
        /// variant. Repeatable and comma-separated. Use `all` to run every declared
        /// variant; the first is the baseline and every
        /// later variant is compared against it. This axis cannot be combined
        /// with `--model`, `--models`, or `--env`.
        #[arg(long = "prompt-variant", value_name = "ID", visible_alias = "prompt-variants")]
        prompt_variants: Vec<String>,
        /// Significance threshold for the paired McNemar verdict rendered by the
        /// `--env` or `--prompt-variant` matrix comparison.
        #[arg(long, value_name = "ALPHA", default_value_t = run_store::DEFAULT_ALPHA)]
        alpha: f64,
        /// SQLite run ledger path. Defaults to CRUCIBLE_DB when set and
        /// non-empty, else the local gitignored run store.
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
    },
    /// Query Crucible's SQLite run ledger.
    Runs {
        #[command(subcommand)]
        command: RunsCommand,
    },
    /// Check whether a declared EvalSpec is an executable contract: every
    /// preflight rule `run` enforces, without needing a runnable corpus
    /// (backlog 014). Exits 0 whether or not the spec is valid — the
    /// `valid`/`runnable` fields carry the verdict; exit 1 is reserved for a
    /// spec that fails to load (unknown schema, malformed JSON).
    Validate {
        /// Path to a declared Crucible EvalSpec JSON.
        #[arg(value_name = "SPEC")]
        spec: PathBuf,
        /// Emit a stable JSON object instead of a human-readable report.
        #[arg(long)]
        json: bool,
    },
    /// Serve Crucible's local benchmark workbench over HTTP on 127.0.0.1.
    Serve {
        /// SQLite run ledger path. Defaults to CRUCIBLE_DB when set and
        /// non-empty, else the local gitignored run store.
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Directory containing declared EvalSpec JSON files.
        #[arg(long, value_name = "DIR", default_value = "evals")]
        specs: PathBuf,
        /// Port to bind. `0` asks the OS for a free port.
        #[arg(long, value_name = "PORT", default_value_t = 4174)]
        port: u16,
    },
    /// Render a phone-first adjudication panel from an existing
    /// `crucible.judgment_queue.v1` queue artifact, optionally serving it with
    /// real writeback (backlog 005).
    AdjudicationPanel {
        /// Path to a judgment queue JSON artifact.
        #[arg(long, value_name = "PATH")]
        queue: PathBuf,
        /// Output directory; `index.html` and a copied `queue.json` are written.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Serve the panel locally with a real `POST /label` writeback loop
        /// instead of only writing static files.
        #[arg(long)]
        serve: bool,
        /// Port to bind for `--serve`. `0` asks the OS for a free port.
        #[arg(long, value_name = "PORT", default_value_t = 4173)]
        port: u16,
        /// Where `--serve` persists applied labels as a `crucible.label.v1`
        /// JSON array. Defaults to `<out>/labels.json`; resumed on restart.
        #[arg(long, value_name = "PATH")]
        labels: Option<PathBuf>,
    },
    /// Serve Crucible's run surface as a stdio Model Context Protocol server.
    Mcp,
    /// Assemble a valid EvalSpec from flags or a guided `--interactive`
    /// prompt flow, validate it the same way `crucible validate` does, and
    /// only write it when valid — no hand-written JSON required
    /// (crucible-942). Boxed: `AuthorArgs` carries every runner kind's flags
    /// at once, making it by far the largest `Command` variant.
    Author(Box<author::AuthorArgs>),
    /// Import an externally-authored eval/benchmark definition, projecting it
    /// into a valid EvalSpec through the same validate-then-save gate
    /// `crucible author` uses (backlog/Powder crucible-026). Nested by
    /// adapter — `promptfoo` is the first and, for now, only one.
    Import {
        #[command(subcommand)]
        adapter: ImportAdapter,
    },
    /// Push-button, verified-live onboarding check (crucible-911): confirms
    /// the CLI runs, MCP initializes and lists tools, `serve` binds a port
    /// and answers `/api/specs`, and the run ledger can be created under
    /// `runs/` — then separately reports whether `OPENROUTER_API_KEY` is
    /// configured (a warning, not a failure, when absent).
    Doctor {
        /// Emit a stable JSON object instead of a human-readable report.
        #[arg(long)]
        json: bool,
    },
    /// Export one stored run as a self-contained public benchmark packet
    /// (`crucible.bench_packet.v1`, crucible-publish-packet) — read-only
    /// against the ledger, refusing rather than emitting a partial packet.
    /// See `publish.rs`.
    Publish {
        /// Stored run id from `crucible runs list`/`show`.
        #[arg(long, value_name = "RUN_ID")]
        run: String,
        /// SQLite run ledger path. Defaults to CRUCIBLE_DB when set and
        /// non-empty, else the local gitignored run store.
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Output directory for the emitted packet JSON file.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum ImportAdapter {
    /// Import a Promptfoo-style YAML eval config into a Crucible
    /// `prompt_benchmark` EvalSpec, run through the same validate-then-save
    /// gate `crucible author` uses. Test cases that cannot be mapped cleanly
    /// (multiple assertions, an unsupported assertion type, an unresolved
    /// `$ref` template or `{{var}}`) are reported, never silently dropped.
    Promptfoo(import::PromptfooImportArgs),
    /// Import a local directory of Harbor task directories into a Crucible
    /// `harbor_task` EvalSpec (backlog/Powder crucible-034), scoped to a
    /// representative CPU-only smoke subset — not the full Terminal-Bench 2.0
    /// dataset, which is a follow-up card. Every directory entry is either
    /// imported or reported as skipped, with why, never silently dropped.
    Harbor(harbor_import::HarborImportArgs),
}

#[derive(Debug, Subcommand)]
enum RunsCommand {
    /// List stored runs, optionally filtered by benchmark, config, model, or
    /// creation date.
    List {
        /// SQLite run ledger path. Defaults to CRUCIBLE_DB when set and
        /// non-empty, else the local gitignored run store.
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Benchmark id to filter on, e.g. prompt-smoke-v0.
        #[arg(long, value_name = "ID")]
        benchmark: Option<String>,
        /// Config id to filter on.
        #[arg(long, value_name = "ID")]
        config: Option<String>,
        /// Model slug to filter on.
        #[arg(long, value_name = "SLUG")]
        model: Option<String>,
        /// Agent harness identity to filter on, e.g. claude-code or codex
        /// (backlog 027).
        #[arg(long, value_name = "HARNESS")]
        harness: Option<String>,
        /// Only runs created at or after this RFC3339 timestamp or YYYY-MM-DD date.
        #[arg(long, value_name = "TIMESTAMP")]
        since: Option<String>,
        /// Only runs created at or before this RFC3339 timestamp or YYYY-MM-DD date.
        #[arg(long, value_name = "TIMESTAMP")]
        until: Option<String>,
        /// Cap the number of rows returned. Omit for every matching row (the
        /// pre-pagination default).
        #[arg(long, value_name = "N")]
        limit: Option<i64>,
        /// Rows to skip before the first returned row; combine with --limit
        /// to page through a large run ledger.
        #[arg(long, value_name = "N")]
        offset: Option<i64>,
        /// Emit stable JSON instead of a readable table.
        #[arg(long)]
        json: bool,
    },
    /// Show one stored run by run id, including artifact pointers and task rows.
    Show {
        /// Stored run id from `crucible runs list`.
        #[arg(value_name = "RUN_ID")]
        run_id: String,
        /// SQLite run ledger path. Defaults to CRUCIBLE_DB when set and
        /// non-empty, else the local gitignored run store.
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Emit stable JSON instead of a readable table.
        #[arg(long)]
        json: bool,
    },
    /// Compare latest stored runs for two configs or model slugs.
    Compare {
        /// SQLite run ledger path. Defaults to CRUCIBLE_DB when set and
        /// non-empty, else the local gitignored run store.
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Benchmark id to compare under.
        #[arg(long, value_name = "ID")]
        benchmark: String,
        /// Left config id or model slug.
        #[arg(long, value_name = "CONFIG_OR_MODEL")]
        left: String,
        /// Right config id or model slug.
        #[arg(long, value_name = "CONFIG_OR_MODEL")]
        right: String,
        /// Significance threshold for the paired McNemar verdict when the two
        /// runs share prompt task fixtures.
        #[arg(long, value_name = "ALPHA", default_value_t = run_store::DEFAULT_ALPHA)]
        alpha: f64,
        /// Require a paired McNemar result over shared task rows. Without this
        /// flag, compare may fall back to a descriptive latest-run delta.
        #[arg(long)]
        paired: bool,
        /// Write a findings journal JSON file. The journal contains a finding
        /// only when this comparison's paired verdict is a statistical signal.
        #[arg(long, value_name = "PATH")]
        findings_out: Option<PathBuf>,
        /// Refuse (rather than caveat) a comparison spanning more than one
        /// identity axis (model, harness, tool_allowlist, scoring) at once —
        /// backlog 974's axis-mismatch guard (SWE-bench-Lite: harness alone
        /// swung 2.7%->28.3% for the same model).
        #[arg(long)]
        strict: bool,
        /// Emit stable JSON instead of a readable table.
        #[arg(long)]
        json: bool,
    },
    /// Query a judge's standing calibration licence by its licence key (the
    /// `licence_key` field on a `crucible.calibration_record.v1`, or the
    /// value logged in an agentic-judge run's notes) — "is this exact judge
    /// (model + prompt + rubric set) currently licensed", across runs,
    /// without recomputing calibration from scratch (backlog 029).
    JudgeStatus {
        /// The calibration record's `licence_key`.
        #[arg(long, value_name = "KEY")]
        licence_key: String,
        /// SQLite run ledger path. Defaults to CRUCIBLE_DB when set and
        /// non-empty, else the local gitignored run store.
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Emit stable JSON instead of a readable table.
        #[arg(long)]
        json: bool,
    },
    /// Time-series score history for one benchmark/config, ordered oldest
    /// to newest (backlog 027) — the longitudinal view a single config's
    /// trend line needs.
    History {
        /// SQLite run ledger path. Defaults to CRUCIBLE_DB when set and
        /// non-empty, else the local gitignored run store.
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Benchmark id to trend.
        #[arg(long, value_name = "ID")]
        benchmark: String,
        /// Config id or model slug to trend.
        #[arg(long, value_name = "CONFIG_OR_MODEL")]
        config: String,
        /// Emit stable JSON instead of a readable table.
        #[arg(long)]
        json: bool,
    },
    /// Cross-axis pivot: one benchmark's latest run per model, optionally
    /// narrowed to one harness (backlog 027) — "this benchmark, this
    /// harness, across all models".
    Pivot {
        /// SQLite run ledger path. Defaults to CRUCIBLE_DB when set and
        /// non-empty, else the local gitignored run store.
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Benchmark id to pivot.
        #[arg(long, value_name = "ID")]
        benchmark: String,
        /// Agent harness identity to narrow to, e.g. claude-code or codex.
        /// Omit to pivot across every harness recorded for the benchmark.
        #[arg(long, value_name = "HARNESS")]
        harness: Option<String>,
        /// Emit stable JSON instead of a readable table.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    // clap owns usage errors (exit 2) and --help/--version (exit 0); everything
    // past parse is a load/parse path that fails with exit 1.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => err.exit(),
    };
    // Install the panic hook and tracing→Canary layer before anything else
    // runs, so a panic or `tracing::error!` anywhere below — including deep
    // inside `serve`/`mcp`'s standing-service loops — is captured. The fmt
    // layer writes to stderr, never stdout: `mcp` uses stdout as its
    // JSON-RPC protocol channel, and log lines on that stream would corrupt
    // it.
    //
    // The `EnvFilter` is a *per-layer* filter on the fmt layer only: it
    // keeps a deployed `crucible serve` from drowning stderr in
    // reqwest/hyper TRACE spam (default `info`, overridable via `RUST_LOG`).
    // `CanaryLayer` is deliberately left unfiltered so it still observes
    // every `ERROR` event regardless of what the console shows.
    canary::install_panic_hook();
    let fmt_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(fmt_filter);
    let _ = tracing_subscriber::registry()
        .with(fmt_layer)
        .with(canary::CanaryLayer)
        .try_init();
    // Fire as early as possible so every invocation is observed, even one
    // that fails deep inside a subcommand below.
    canary::check_in();
    let result = match cli.command {
        Command::Adapt { artifact, json } => run_adapt(&artifact, json),
        Command::Grade {
            artifact,
            key,
            json,
        } => run_grade(&artifact, &key, json),
        Command::Adjudicate {
            artifact,
            key,
            apply,
            json,
        } => run_adjudicate(&artifact, &key, apply.as_deref(), json),
        Command::Export {
            labels,
            out,
            arena,
            task,
            base_version,
            date,
            key,
            expected,
        } => run_export(&ExportRequest {
            labels: &labels,
            out: &out,
            arena: &arena,
            task: &task,
            base_version: &base_version,
            date: &date,
            key: key.as_deref(),
            expected: expected.as_deref(),
        }),
        Command::Dashboard { arenas, runs, out } => run_dashboard(&arenas, &runs, &out),
        Command::Run {
            spec,
            eval,
            out,
            json,
            strict_tracked,
            model,
            models,
            envs,
            prompt_variants,
            alpha,
            db,
        } => run_eval(
            spec.as_deref(),
            eval,
            out.as_deref(),
            json,
            strict_tracked,
            model.as_deref(),
            models.as_deref(),
            &envs,
            &prompt_variants,
            alpha,
            &db.unwrap_or_else(run_store::default_db_path),
        ),
        Command::Runs { command } => run_runs(command),
        Command::Validate { spec, json } => run_validate(&spec, json),
        Command::Serve { db, specs, port } => serve::serve(serve::ServeOptions {
            db_path: db.unwrap_or_else(run_store::default_db_path),
            specs_dir: specs,
            port,
        }),
        Command::AdjudicationPanel {
            queue,
            out,
            serve,
            port,
            labels,
        } => run_adjudication_panel(&queue, &out, serve, port, labels.as_deref()),
        Command::Mcp => mcp::run_stdio(),
        Command::Author(args) => author::run(*args),
        Command::Import { adapter } => match adapter {
            ImportAdapter::Promptfoo(args) => import::run(args),
            ImportAdapter::Harbor(args) => harbor_import::run(args),
        },
        Command::Doctor { json } => run_doctor(json),
        Command::Publish { run, db, out } => {
            run_publish(&run, &db.unwrap_or_else(run_store::default_db_path), &out)
        }
    };
    let exit = match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            canary::report_error("crucible.run.failed", &format!("{err:#}"));
            ExitCode::from(EXIT_LOAD_ERROR)
        }
    };
    // Give the check-in and/or error report a bounded window to reach the
    // network before this short-lived process exits.
    canary::flush();
    exit
}

/// `crucible run`: execute a declared spec when supplied, otherwise run built-in
/// eval receipts.
#[allow(clippy::too_many_arguments)]
fn run_eval(
    spec: Option<&Path>,
    eval: eval_run::RunEval,
    out: Option<&Path>,
    json: bool,
    strict_tracked: bool,
    model: Option<&str>,
    models: Option<&str>,
    envs: &[PathBuf],
    prompt_variants: &[String],
    alpha: f64,
    db: &Path,
) -> anyhow::Result<()> {
    let override_flags = [
        model.is_some(),
        models.is_some(),
        !envs.is_empty(),
        !prompt_variants.is_empty(),
    ];
    if override_flags.iter().filter(|set| **set).count() > 1 {
        anyhow::bail!("--model, --models, --env, and --prompt-variant are mutually exclusive");
    }
    if !envs.is_empty() {
        if eval != eval_run::RunEval::All {
            anyhow::bail!("--env selects a declared spec and cannot be combined with --eval");
        }
        return run_matrix::run(spec, out, json, strict_tracked, envs, alpha, db);
    }
    if !prompt_variants.is_empty() {
        if eval != eval_run::RunEval::All {
            anyhow::bail!("--prompt-variant selects a declared spec and cannot be combined with --eval");
        }
        return run_prompt_variants::run(
            spec,
            out,
            json,
            strict_tracked,
            prompt_variants,
            alpha,
            db,
        );
    }
    if let Some(models) = models {
        return run_fanout::run(spec, eval, out, json, strict_tracked, models, db);
    }
    let report = if let Some(spec_path) = spec {
        if eval != eval_run::RunEval::All {
            anyhow::bail!(
                "--eval selects built-in receipts and cannot be combined with a spec path"
            );
        }
        let options = match model {
            Some(model) => spec_run::RunOptions::with_prompt_model(model),
            None => spec_run::RunOptions::default(),
        };
        spec_run::run_with_options(spec_path, out, &options)?
    } else {
        if model.is_some() {
            anyhow::bail!("--model can only be used with a declared prompt_benchmark spec");
        }
        let out = out.with_context(|| "built-in receipt runs require --out <DIR>")?;
        eval_run::run(eval, out)?
    };
    let stored = run_store::persist_report(db, &report)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("crucible run");
        println!("  out      {}", report.output_dir);
        for eval in &report.evals {
            println!(
                "  eval     {}  {}",
                eval.id,
                eval_run::format_score(&eval.score)
            );
        }
        println!(
            "  wrote    {}",
            Path::new(&report.output_dir)
                .join("run-report.json")
                .display()
        );
        println!(
            "  stored   {}  ({} run row{}, {} prompt task row{}, {} harbor task row{})",
            stored.db,
            stored.run_records,
            plural(stored.run_records),
            stored.prompt_task_results,
            plural(stored.prompt_task_results),
            stored.harbor_task_results,
            plural(stored.harbor_task_results)
        );
    }
    if strict_tracked {
        let failures = spec_run::tracked_failures(&report)?;
        if !failures.is_empty() {
            anyhow::bail!(
                "tracked checks failed: {}",
                spec_run::format_tracked_failures(&failures)
            );
        }
    }
    Ok(())
}

/// `crucible validate`: is a declared spec an executable contract?
fn run_validate(spec: &Path, json: bool) -> anyhow::Result<()> {
    let report = validate::validate(spec)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_validation_report(&report);
    }
    Ok(())
}

fn print_validation_report(report: &validate::ValidationReport) {
    println!("crucible validate");
    if let Some(title) = &report.title {
        println!("  title     {title}");
    }
    println!("  spec      {}", report.spec);
    println!("  valid     {}", report.valid);
    println!("  runnable  {}", report.runnable);
    for error in &report.errors {
        println!("  ERROR     {}: {}", error.field, error.message);
    }
    for warning in &report.warnings {
        println!("  warning   {}: {}", warning.field, warning.message);
    }
    if report.errors.is_empty() && report.warnings.is_empty() {
        println!("  (no issues)");
    }
}

/// `crucible doctor`: run every onboarding self-check, print the full report
/// (JSON or human-readable) unconditionally, then fail the process — exit `1`
/// via the caller's error mapping — iff any check is broken. Missing optional
/// model credentials alone never triggers this; only a real `Fail` check does.
fn run_doctor(json: bool) -> anyhow::Result<()> {
    let report = doctor::run();
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_doctor_report(&report);
    }
    if report.ok {
        return Ok(());
    }
    let failed: Vec<&str> = report
        .checks
        .iter()
        .filter(|check| check.status == doctor::CheckStatus::Fail)
        .map(|check| check.id)
        .collect();
    anyhow::bail!(
        "doctor found {} broken check{}: {}",
        failed.len(),
        plural(failed.len()),
        failed.join(", ")
    );
}

/// `crucible publish`: export one stored run as a `crucible.bench_packet.v1`
/// JSON file. See `publish.rs` for the refusal contract.
fn run_publish(run_id: &str, db: &Path, out: &Path) -> anyhow::Result<()> {
    let packet_path = publish::publish(run_id, db, out)?;
    println!("crucible publish");
    println!("  run       {run_id}");
    println!("  db        {}", db.display());
    println!("  wrote     {}", packet_path.display());
    Ok(())
}

fn print_doctor_report(report: &doctor::DoctorReport) {
    println!("crucible doctor");
    println!("  ok        {}", report.ok);
    for check in &report.checks {
        let status = match check.status {
            doctor::CheckStatus::Ok => "ok",
            doctor::CheckStatus::Warn => "warn",
            doctor::CheckStatus::Fail => "FAIL",
        };
        println!("  {:<9} {:<18} {}", status, check.id, check.message);
    }
}

fn run_runs(command: RunsCommand) -> anyhow::Result<()> {
    match command {
        RunsCommand::List {
            db,
            benchmark,
            config,
            model,
            harness,
            since,
            until,
            limit,
            offset,
            json,
        } => {
            let db = db.unwrap_or_else(run_store::default_db_path);
            let since_unix_ms = since
                .as_deref()
                .map(run_store::parse_timestamp_bound)
                .transpose()?;
            let until_unix_ms = until
                .as_deref()
                .map(run_store::parse_timestamp_bound)
                .transpose()?;
            let filter = run_store::RunListFilter {
                benchmark: benchmark.as_deref(),
                config: config.as_deref(),
                model: model.as_deref(),
                harness: harness.as_deref(),
                since_unix_ms,
                until_unix_ms,
                limit,
                offset,
            };
            let list = run_store::list_runs(&db, filter)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else {
                print_run_list(&list);
            }
        }
        RunsCommand::Show { run_id, db, json } => {
            let db = db.unwrap_or_else(run_store::default_db_path);
            let detail = run_store::show_run(&db, &run_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&detail)?);
            } else {
                print_run_detail(&detail);
            }
        }
        RunsCommand::Compare {
            db,
            benchmark,
            left,
            right,
            alpha,
            paired,
            findings_out,
            strict,
            json,
        } => {
            let db = db.unwrap_or_else(run_store::default_db_path);
            let comparison =
                run_store::compare_configs(&db, &benchmark, &left, &right, alpha, strict)?;
            if paired && comparison.paired.is_none() {
                anyhow::bail!("--paired requested but the two runs share no comparable task rows");
            }
            let findings_receipt = if let Some(path) = findings_out.as_deref() {
                let repro_command =
                    runs_compare_repro_command(&db, &benchmark, &left, &right, alpha);
                Some(findings_journal::write_journal(
                    &comparison,
                    alpha,
                    repro_command,
                    path,
                )?)
            } else {
                None
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&comparison)?);
            } else {
                print_config_comparison(&comparison);
                if let (Some(path), Some(journal)) = (findings_out.as_deref(), findings_receipt) {
                    println!(
                        "  findings {}  ({} record{})",
                        path.display(),
                        journal.findings.len(),
                        plural(journal.findings.len())
                    );
                }
            }
        }
        RunsCommand::JudgeStatus {
            licence_key,
            db,
            json,
        } => {
            let db = db.unwrap_or_else(run_store::default_db_path);
            let status = run_store::judge_licence_status(&db, &licence_key)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print_judge_licence_status(&licence_key, status.as_ref());
            }
        }
        RunsCommand::History {
            db,
            benchmark,
            config,
            json,
        } => {
            let db = db.unwrap_or_else(run_store::default_db_path);
            let history = run_store::score_history(&db, &benchmark, &config)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&history)?);
            } else {
                print_score_history(&history);
            }
        }
        RunsCommand::Pivot {
            db,
            benchmark,
            harness,
            json,
        } => {
            let db = db.unwrap_or_else(run_store::default_db_path);
            let pivot = run_store::pivot_by_model(&db, &benchmark, harness.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&pivot)?);
            } else {
                print_pivot_view(&pivot);
            }
        }
    }
    Ok(())
}

fn print_judge_licence_status(licence_key: &str, status: Option<&run_store::JudgeLicenceStatus>) {
    println!("crucible runs judge-status");
    println!("  licence_key  {licence_key}");
    match status {
        None => println!(
            "  (no run has measured this judge/prompt/rubric identity — locked/unlicensed)"
        ),
        Some(status) => {
            println!("  judge_model  {}", status.judge_model);
            println!(
                "  unlocked     {}  (agreement {:.2}, κ {:.2}, threshold {:.2})",
                status.unlocked, status.agreement, status.cohen_kappa, status.unlock_threshold
            );
            println!(
                "  fp_rate      {:.2}   fn_rate  {:.2}",
                status.false_positive_rate, status.false_negative_rate
            );
            if status.self_evaluation_bias_risk {
                println!(
                    "  bias_risk    SELF-EVALUATION RISK (generator {})",
                    status.generator_id.as_deref().unwrap_or("?")
                );
            }
            println!("  n            {}", status.n);
            println!("  from_run     {}", status.run_id);
            println!("  updated_at   {}", status.updated_at_unix_ms);
        }
    }
}

fn print_run_list(list: &run_store::RunList) {
    println!("crucible runs list");
    println!("  db        {}", list.db);
    if let Some(benchmark) = &list.benchmark {
        println!("  benchmark {benchmark}");
    }
    if let Some(config) = &list.config {
        println!("  config    {config}");
    }
    if let Some(model) = &list.model {
        println!("  model     {model}");
    }
    if let Some(since) = list.since_unix_ms {
        println!("  since     {since}");
    }
    if let Some(until) = list.until_unix_ms {
        println!("  until     {until}");
    }
    if list.runs.is_empty() {
        println!("  (no runs)");
        return;
    }
    for run in &list.runs {
        println!(
            "  run       {}  {}  config={}  {}",
            run.run_id,
            run.benchmark_id,
            run.config_id,
            format_stored_score(run)
        );
    }
}

fn print_run_detail(detail: &run_store::RunDetail) {
    let run = &detail.run;
    println!("crucible runs show");
    println!("  db        {}", detail.db);
    println!("  run       {}", run.run_id);
    println!("  benchmark {}", run.benchmark_id);
    println!("  config    {}", run.config_id);
    if let Some(model) = &run.model {
        println!("  model     {model}");
    }
    println!("  score     {}", format_stored_score(run));
    println!("  report    {}", run.run_report);
    for artifact in &detail.artifacts {
        println!("  artifact  {}  ({})", artifact.path, artifact.kind);
    }
    if !detail.prompt_tasks.is_empty() {
        println!("  prompt task rows {}", detail.prompt_tasks.len());
        for task in &detail.prompt_tasks {
            if task.tracked_results.is_empty() {
                continue;
            }
            let outcomes = task
                .tracked_results
                .iter()
                .map(|check| {
                    format!(
                        "{}={}",
                        check.id,
                        if check.passed { "pass" } else { "fail" }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            println!("  tracked  {}  {}", task.task_id, outcomes);
        }
    }
    if !detail.harbor_tasks.is_empty() {
        println!("  harbor task rows {}", detail.harbor_tasks.len());
    }
}

pub(crate) fn print_config_comparison(comparison: &run_store::ConfigComparison) {
    println!("crucible runs compare");
    println!("  db        {}", comparison.db);
    println!("  benchmark {}", comparison.benchmark);
    println!(
        "  left      {}  {}",
        comparison.left_query,
        format_stored_score(&comparison.left)
    );
    println!(
        "  right     {}  {}",
        comparison.right_query,
        format_stored_score(&comparison.right)
    );
    match comparison.delta_point {
        Some(delta) => println!("  delta     {delta:+.4}"),
        None => println!("  delta     n/a"),
    }
    println!("  kind      {}", comparison.comparison_kind);
    println!("  attrib    {}", comparison.attribution);
    if let Some(note) = &comparison.attribution_note {
        println!("  {note}");
    }
    if let Some(caveat) = &comparison.resource_envelope_caveat {
        println!("  {caveat}");
    }
    if let Some(paired) = &comparison.paired {
        println!(
            "  paired    n={}  b={}  c={}  chi2={:.4}  p={:.4}  {}",
            comparison.common_tasks,
            paired.b,
            paired.c,
            paired.statistic,
            paired.p_value,
            paired.verdict.label()
        );
    }
    if let Some(resolution) = &comparison.resolution {
        println!("  {}", format_resolution_line(resolution));
    }
    if !comparison.class_breakdowns.is_empty() {
        println!("  classes");
        for row in &comparison.class_breakdowns {
            let delta = row
                .delta_point
                .map(|delta| format!("{delta:+.4}"))
                .unwrap_or_else(|| "n/a".to_string());
            let verdict = row
                .paired
                .as_ref()
                .map(|paired| paired.verdict.label().to_string())
                .unwrap_or_else(|| "unpaired".to_string());
            println!(
                "    {:<26} left={}/{} right={}/{} delta={} paired_n={} {}",
                row.class,
                row.left_successes,
                row.left_n,
                row.right_successes,
                row.right_n,
                delta,
                row.common_tasks,
                verdict
            );
            if let Some(resolution) = &row.resolution {
                println!("      {}", format_resolution_line(resolution));
            }
        }
    }
    println!("  note      {}", comparison.note);
}

/// One line reporting Kotawala's resolution ratio and MDE (arXiv:2605.30315)
/// beside a paired comparison's `DeltaVerdict` — see `docs/design-references.md`
/// §1 for why an underpowered comparison can look identical to a genuine
/// "no difference" verdict.
fn format_resolution_line(resolution: &run_store::PowerResolution) -> String {
    let q = resolution
        .resolution_ratio
        .map(|q| format!("{q:.2}"))
        .unwrap_or_else(|| "n/a".to_string());
    let required_n = resolution
        .required_n
        .map(|n| n.to_string())
        .unwrap_or_else(|| "n/a".to_string());
    let mde = resolution
        .minimum_detectable_effect
        .map(|mde| format!("{mde:.4}"))
        .unwrap_or_else(|| "n/a".to_string());
    format!(
        "resolution q={q} (required_n={required_n})  mde={mde}  (alpha={:.2}, power={:.2})  {}",
        resolution.alpha, resolution.power, resolution.diagnosis
    )
}

fn print_score_history(history: &run_store::ScoreHistory) {
    println!("crucible runs history");
    println!("  db        {}", history.db);
    println!("  benchmark {}", history.benchmark);
    println!("  config    {}", history.config_query);
    if history.points.is_empty() {
        println!("  (no runs)");
        return;
    }
    for point in &history.points {
        let score = match point.point {
            Some(value) => format!(
                "{:.1}%   {:.0}% CI [{:.1}%, {:.1}%]   ({}; {}/{})",
                value * 100.0,
                point.confidence * 100.0,
                point.lower * 100.0,
                point.upper * 100.0,
                point.method,
                point.successes,
                point.n
            ),
            None => format!(
                "n/a   {:.0}% CI [{:.1}%, {:.1}%]   ({}; {}/{})",
                point.confidence * 100.0,
                point.lower * 100.0,
                point.upper * 100.0,
                point.method,
                point.successes,
                point.n
            ),
        };
        println!(
            "  {}  run={}  {}",
            point.created_at_unix_ms, point.run_id, score
        );
    }
}

fn print_pivot_view(pivot: &run_store::PivotView) {
    println!("crucible runs pivot");
    println!("  db        {}", pivot.db);
    println!("  benchmark {}", pivot.benchmark);
    match &pivot.harness {
        Some(harness) => println!("  harness   {harness}"),
        None => println!("  harness   (all)"),
    }
    if pivot.rows.is_empty() {
        println!("  (no runs)");
        return;
    }
    for row in &pivot.rows {
        println!(
            "  model={:<28} {}",
            row.model.as_deref().unwrap_or("(unknown)"),
            format_stored_score(&row.latest_run)
        );
    }
}

fn format_stored_score(run: &run_store::StoredRun) -> String {
    match run.point {
        Some(point) => format!(
            "{:.1}%   {:.0}% CI [{:.1}%, {:.1}%]   ({}; {}/{})",
            point * 100.0,
            run.confidence * 100.0,
            run.lower * 100.0,
            run.upper * 100.0,
            run.method,
            run.successes,
            run.n
        ),
        None => format!(
            "n/a   {:.0}% CI [{:.1}%, {:.1}%]   ({}; {}/{})",
            run.confidence * 100.0,
            run.lower * 100.0,
            run.upper * 100.0,
            run.method,
            run.successes,
            run.n
        ),
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// The `crucible runs compare` invocation that reproduces one comparison.
///
/// Shared by the CLI, the MCP `crucible_runs_compare` tool, and `crucible
/// serve`'s `/api/compare` route so every face's findings journal points at
/// the same repro command for the same comparison.
pub(crate) fn runs_compare_repro_command(
    db: &Path,
    benchmark: &str,
    left: &str,
    right: &str,
    alpha: f64,
) -> String {
    format!(
        "crucible runs compare --db {} --benchmark {} --left {} --right {} --alpha {} --json",
        shell_word(&db.display().to_string()),
        shell_word(benchmark),
        shell_word(left),
        shell_word(right),
        alpha
    )
}

fn shell_word(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// `crucible adjudication-panel`: render a phone-first static HTML panel from an
/// existing judgment queue, or (`--serve`) serve it with real writeback.
fn run_adjudication_panel(
    queue: &Path,
    out: &Path,
    serve: bool,
    port: u16,
    labels: Option<&Path>,
) -> anyhow::Result<()> {
    let receipt = adjudication_panel::write_panel(queue, out)?;
    println!("crucible adjudication-panel");
    println!("  queue    {}", queue.display());
    println!("  items    {}", receipt.items);
    println!("  labels   {}", receipt.labels);
    println!("  wrote    {}", receipt.html_path.display());
    println!("  wrote    {}", receipt.queue_path.display());
    if serve {
        let labels_path = labels
            .map(Path::to_path_buf)
            .unwrap_or_else(|| out.join("labels.json"));
        adjudication_server::serve(adjudication_server::ServeOptions {
            queue_path: queue.to_path_buf(),
            labels_path,
            port,
        })?;
    }
    Ok(())
}

/// `crucible adapt`: map every finding in the artifact and print the rows.
fn run_adapt(artifact: &Path, json: bool) -> anyhow::Result<()> {
    let findings = findings_from_artifact(artifact)
        .with_context(|| format!("loading artifact {}", artifact.display()))?;
    let rows = to_key_findings(&findings);

    if json {
        let report = AdaptReport {
            schema_version: ADAPT_REPORT_SCHEMA,
            artifact: artifact.display().to_string(),
            count: rows.len(),
            findings: &rows,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_adapt_table(artifact, &rows);
    }
    Ok(())
}

/// `crucible grade`: run the deterministic pre-grader and report the result.
fn run_grade(artifact: &Path, key_path: &Path, json: bool) -> anyhow::Result<()> {
    let report = build_grade_report(artifact, key_path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_grade_summary(&report);
    }
    Ok(())
}

/// Run the deterministic pre-grader and assemble the stable [`GradeReport`] —
/// the one computation both `crucible grade` and MCP `crucible_grade` call, so
/// neither re-implements the other.
fn build_grade_report(artifact: &Path, key_path: &Path) -> anyhow::Result<GradeReport> {
    let (candidates, dropped_invalid) = candidate_rows(artifact)?;
    let key_rows = load_key_rows(key_path)?;

    let result = grade(&candidates, &key_rows);
    let match_rate = MatchRate::from_grade(&result);
    let recoverable = recoverable_misses(&result);

    Ok(GradeReport {
        schema_version: GRADE_REPORT_SCHEMA,
        artifact: artifact.display().to_string(),
        key: key_path.display().to_string(),
        matched: result.matched.len(),
        disputed: result.disputed.len(),
        missed: result.missed.len(),
        dropped_invalid,
        recoverable_misses: recoverable,
        match_rate,
    })
}

/// `crucible adjudicate`: grade, build the queue, optionally apply labels, emit.
fn run_adjudicate(
    artifact: &Path,
    key_path: &Path,
    apply: Option<&Path>,
    json: bool,
) -> anyhow::Result<()> {
    let queue = build_judgment_queue(artifact, key_path, apply)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&queue)?);
    } else {
        print_queue(artifact, key_path, &queue);
    }
    Ok(())
}

/// Grade, build the adjudication queue, and optionally apply labels — the one
/// computation both `crucible adjudicate` and MCP `crucible_adjudicate` call.
fn build_judgment_queue(
    artifact: &Path,
    key_path: &Path,
    apply: Option<&Path>,
) -> anyhow::Result<JudgmentQueue> {
    let (candidates, _dropped) = candidate_rows(artifact)?;
    let key_rows = load_key_rows(key_path)?;
    let result = grade(&candidates, &key_rows);
    let mut queue = build_queue(&result);

    if let Some(apply_path) = apply {
        queue.labels = mint_labels(&queue, &load_decisions(apply_path)?)?;
    }
    Ok(queue)
}

/// The inputs `crucible export` works from: the labeled queue, the output dir,
/// the arena/task/date/version document context, and the optional original keys
/// to extend. Grouped into one request so the call reads as a single intent
/// rather than a long positional argument list.
struct ExportRequest<'a> {
    labels: &'a Path,
    out: &'a Path,
    arena: &'a str,
    task: &'a str,
    base_version: &'a str,
    date: &'a str,
    key: Option<&'a Path>,
    expected: Option<&'a Path>,
}

/// `crucible export`: turn a labeled judgment queue into the Daedalus
/// key-extension artifacts.
///
/// Always writes `<out>/adjudications.md` (the human key-extension log). With
/// `--key`, also writes `<out>/solution/findings.json` — the point oracle
/// extended with the accepted findings. With `--expected`, also writes
/// `<out>/tests/expected.json` — the line-span scorer key `daedalus-score`
/// reads, extended so an accepted finding re-scores as a true positive instead
/// of a false positive. The version bump for each ACCEPT walks forward from
/// `--base-version`.
fn run_export(req: &ExportRequest<'_>) -> anyhow::Result<()> {
    let report = build_export(req)?;
    println!("crucible export");
    println!("  arena         {}", report.arena);
    println!("  task          {}", report.task);
    println!(
        "  adjudications {}  ({} accept, {} out-of-scope)",
        report.adjudications, report.accepts, report.out_of_scope,
    );
    println!("  wrote         {}", report.adjudications_md);
    if let Some(p) = &report.oracle_key {
        println!("  oracle key    {p}");
    }
    if let Some(p) = &report.scorer_key {
        println!("  scorer key    {p}");
    }
    Ok(())
}

/// Turn a labeled judgment queue into the Daedalus key-extension artifacts and
/// assemble the stable [`ExportReport`] — the one computation both
/// `crucible export` and MCP `crucible_export` call, writes and all.
fn build_export(req: &ExportRequest<'_>) -> anyhow::Result<ExportReport> {
    let &ExportRequest {
        labels,
        out,
        arena,
        task,
        base_version,
        date,
        key,
        expected,
    } = req;
    let queue = load_queue(labels)?;
    let base_version: ArenaVersion = base_version
        .parse()
        .with_context(|| format!("parsing --base-version {base_version:?}"))?;
    let ctx = ExportContext {
        arena: arena.to_string(),
        task: task.to_string(),
        date: date.to_string(),
        base_version,
    };

    let adjudications = adjudications_from_queue(&queue, &ctx)?;

    // Render and serialize EVERY output before writing anything. A bad --key or
    // --expected (missing, malformed) then fails fast — it never leaves a
    // half-written tree whose adjudications.md asserts an ACCEPT/version bump that
    // never landed. The only failures left at write time are I/O, after every
    // input has parsed.
    let md_path = out.join("adjudications.md");
    let mut outputs: Vec<(PathBuf, String)> = vec![(
        md_path.clone(),
        render_adjudications_md(&ctx.arena, &adjudications),
    )];

    let key_path = match key {
        Some(original_key) => {
            let original = AnswerKey::from_path(original_key)
                .with_context(|| format!("loading original key {}", original_key.display()))?;
            let extended = extended_key(&original, &adjudications);
            let findings_path = out.join("solution").join("findings.json");
            outputs.push((
                findings_path.clone(),
                format!("{}\n", serde_json::to_string_pretty(&extended)?),
            ));
            Some(findings_path)
        }
        None => None,
    };

    let expected_path = match expected {
        Some(original_expected) => {
            let original = ExpectedKey::from_path(original_expected).with_context(|| {
                format!(
                    "loading original scorer key {}",
                    original_expected.display()
                )
            })?;
            let extended = extended_expected_key(&original, &adjudications);
            let expected_out = out.join("tests").join("expected.json");
            outputs.push((
                expected_out.clone(),
                format!("{}\n", serde_json::to_string_pretty(&extended)?),
            ));
            Some(expected_out)
        }
        None => None,
    };

    // Inputs validated and outputs rendered — commit the writes together.
    for (path, content) in &outputs {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    }

    let accepts = adjudications.iter().filter(|a| a.is_accept()).count();
    Ok(ExportReport {
        schema_version: EXPORT_REPORT_SCHEMA,
        arena: arena.to_string(),
        task: task.to_string(),
        adjudications: adjudications.len(),
        accepts,
        out_of_scope: adjudications.len() - accepts,
        adjudications_md: md_path.display().to_string(),
        oracle_key: key_path.map(|p| p.display().to_string()),
        scorer_key: expected_path.map(|p| p.display().to_string()),
    })
}

/// Stable report for `crucible export --json` and MCP `crucible_export`.
#[derive(Debug, Serialize)]
struct ExportReport {
    schema_version: &'static str,
    arena: String,
    task: String,
    adjudications: usize,
    accepts: usize,
    out_of_scope: usize,
    adjudications_md: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    oracle_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scorer_key: Option<String>,
}

/// `crucible dashboard`: ingest the real arenas + runs, measure them, and write
/// the self-contained HTML dashboard and its `data.json` model under `out`.
///
/// The [`Dataset`] loader is total (it never errors), so the only failure paths
/// here are operational: a `runs` path that is not a directory — surfaced up front
/// rather than silently rendering an empty dashboard — or an I/O error creating
/// `out` or writing an artifact. Both exit `1` via the caller's error mapping.
/// `data.json` is the full, stable model (the same [`Dataset`] + [`Leaderboard`]
/// the page renders); `index.html` recomputes no statistic, only displays them.
fn run_dashboard(arenas: &Path, runs: &Path, out: &Path) -> anyhow::Result<()> {
    if !runs.is_dir() {
        anyhow::bail!(
            "runs path {} is not a directory — point --runs at a Daedalus runs/ tree",
            runs.display()
        );
    }

    let dataset = Dataset::load(arenas, runs);
    let leaderboard = Leaderboard::from_dataset(&dataset);
    let run_details = dashboard_html::run_details(runs);
    let data = dashboard_html::DashboardData {
        schema_version: dashboard_html::DASHBOARD_SCHEMA,
        arenas_dir: arenas.display().to_string(),
        runs_dir: runs.display().to_string(),
        dataset: &dataset,
        leaderboard: &leaderboard,
        run_details: &run_details,
    };

    std::fs::create_dir_all(out)
        .with_context(|| format!("creating output directory {}", out.display()))?;

    let data_path = out.join("data.json");
    let json = serde_json::to_string_pretty(&data).context("serializing the dashboard model")?;
    std::fs::write(&data_path, format!("{json}\n"))
        .with_context(|| format!("writing {}", data_path.display()))?;

    let html_path = out.join("index.html");
    std::fs::write(&html_path, dashboard_html::render(&data))
        .with_context(|| format!("writing {}", html_path.display()))?;

    println!("crucible dashboard");
    println!("  arenas   {}", arenas.display());
    println!("  runs     {}", runs.display());
    println!("  evals    {}", dataset.group_count());
    println!("  runs     {}", dataset.runs.len());
    println!("  trials   {}", dataset.trial_count());
    if dataset.skipped > 0 {
        println!(
            "  skipped  {}  (unparseable/unplaceable trial lines)",
            dataset.skipped
        );
    }
    if !dataset.skipped_inputs.is_empty() {
        let count = |reason: SkipReason| {
            dataset
                .skipped_inputs
                .iter()
                .filter(|s| s.reason == reason)
                .count()
        };
        println!(
            "  skipped inputs  {}  ({} no-placeable-trials · {} unsupported-format · {} no-trials-file)",
            dataset.skipped_inputs.len(),
            count(SkipReason::NoPlaceableTrials),
            count(SkipReason::UnsupportedFormat),
            count(SkipReason::NoTrialsFile),
        );
    }
    println!("  wrote    {}", html_path.display());
    println!("  wrote    {}", data_path.display());
    Ok(())
}

/// Read a labeled judgment queue (the `adjudicate --apply` artifact) from disk.
fn load_queue(path: &Path) -> anyhow::Result<JudgmentQueue> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading labels file {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {} as a labeled judgment queue", path.display()))
}

/// Candidate side of a grade: load findings, drop schema-invalid ones (per the
/// [`grade()`] contract — it does no filtering itself), then project the survivors
/// onto answer-key rows. Returns the rows and how many findings the validity
/// filter dropped, so the report can distinguish "graded nothing" from "the
/// review emitted only malformed findings".
fn candidate_rows(artifact: &Path) -> anyhow::Result<(Vec<KeyFinding>, usize)> {
    let findings = findings_from_artifact(artifact)
        .with_context(|| format!("loading artifact {}", artifact.display()))?;
    let total = findings.len();
    let valid: Vec<_> = findings.into_iter().filter(schema_valid).collect();
    let dropped = total - valid.len();
    Ok((to_key_findings(&valid), dropped))
}

/// Load the answer key and dedup its rows — the prepared key side of a grade.
///
/// Accepts a Daedalus key in *either* real shape: the `solution/findings.json`
/// point oracle (`{ "findings": [...] }`) or the `tests/expected.json` span
/// scorer key (`{ "defects": [...] }`, the file `daedalus-score` reads). A
/// defects file is projected onto rows by [`Defect::to_key_finding`]
/// (`line = line_start`). A file carrying neither array is a hard error, not a
/// silent zero-row grade: grading an unrecognized key would surface a `0%` match
/// rate that is really "the key never loaded".
fn load_key_rows(key_path: &Path) -> anyhow::Result<Vec<KeyFinding>> {
    let bytes = std::fs::read(key_path)
        .with_context(|| format!("reading answer key {}", key_path.display()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing answer key {} as JSON", key_path.display()))?;

    // A bare top-level array (e.g. a labels file or a raw findings list passed by
    // mistake) has no 'findings'/'defects' key and would otherwise fall through to
    // the generic "is this a Daedalus answer key?" error. Name the structural
    // mismatch directly so it is never silently graded against zero key rows.
    if value.is_array() {
        anyhow::bail!(
            "key {} is a top-level JSON array, but a Daedalus answer key is an object with a 'findings' (solution/findings.json) or 'defects' (tests/expected.json) array",
            key_path.display()
        );
    }

    let rows = if value.get("findings").is_some() {
        serde_json::from_value::<AnswerKey>(value)
            .with_context(|| format!("loading solution/findings.json key {}", key_path.display()))?
            .findings
    } else if value.get("defects").is_some() {
        serde_json::from_value::<ExpectedKey>(value)
            .with_context(|| format!("loading tests/expected.json key {}", key_path.display()))?
            .defects
            .iter()
            .map(Defect::to_key_finding)
            .collect()
    } else {
        anyhow::bail!(
            "key {} has no 'findings' array (solution/findings.json) and no 'defects' array (tests/expected.json) — is this a Daedalus answer key?",
            key_path.display()
        );
    };
    Ok(dedup(rows))
}

/// Read an `--apply` file: a JSON array of [`Label`] decisions a judge committed.
fn load_decisions(path: &Path) -> anyhow::Result<Vec<Label>> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading labels file {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "parsing labels file {} as a JSON array of labels",
            path.display()
        )
    })
}

/// Validate each decision against the queue and re-mint it through
/// [`apply_label`], so every emitted label is canonical (current schema, finding
/// id taken from a real queue item). A decision naming a finding that is not an
/// adjudication item is a hard error — it would be a judgment about a finding the
/// deterministic floor already resolved, or one that does not exist.
///
/// The batch is first [`reconcile_labels`]d: duplicate `finding_id`s collapse to
/// the one latest decision (append-only, last-write-wins), so a finding re-decided
/// in a single `--apply` is corrected, not double-applied.
fn mint_labels(queue: &JudgmentQueue, decisions: &[Label]) -> anyhow::Result<Vec<Label>> {
    reconcile_labels(decisions)
        .into_iter()
        .map(|d| {
            let item = queue.item(&d.finding_id).with_context(|| {
                format!(
                    "label references finding id {:?}, which is not an adjudication item in this queue",
                    d.finding_id
                )
            })?;
            let conditions = LabelConditions {
                latency_ms: d.latency_ms,
                saw_grader_before_commit: d.saw_grader_before_commit,
                timestamp: d.timestamp.clone(),
            };
            Ok(apply_label(item, d.verdict, d.disposition, &conditions))
        })
        .collect()
}

/// The match-rate point estimate and its Wilson interval, with the raw counts
/// kept so a consumer can tell a true zero rate apart from "no key rows".
#[derive(Debug, Serialize)]
struct MatchRate {
    /// Numerator: matched count.
    successes: u64,
    /// Denominator: `matched + missed`.
    n: u64,
    /// Point estimate `successes / n`, or `null` when `n == 0` — "no key rows to
    /// match" is not a 0% rate, and a consumer must not read it as one.
    point: Option<f64>,
    /// Lower Wilson bound.
    lower: f64,
    /// Upper Wilson bound.
    upper: f64,
    /// Standard-normal quantile used for the interval.
    z: f64,
    /// Confidence level `z` corresponds to.
    confidence: f64,
}

impl MatchRate {
    fn from_grade(result: &GradeResult) -> Self {
        let successes = result.matched.len() as u64;
        let n = successes + result.missed.len() as u64;
        let (lower, upper) = wilson_interval(successes, n, Z_95);
        MatchRate {
            successes,
            n,
            point: (n != 0).then(|| proportion(successes, n)),
            lower,
            upper,
            z: Z_95,
            confidence: CONFIDENCE,
        }
    }
}

/// Build the same Wilson-shaped score used by `grade` for non-grade binary eval
/// receipts. Kept in the CLI layer so built-in evals do not fork the interval
/// math or silently report pass/fail.
fn wilson_score(metric: &'static str, successes: u64, n: u64) -> eval_run::Score {
    let (lower, upper) = wilson_interval(successes, n, Z_95);
    eval_run::Score {
        metric,
        successes,
        n,
        point: (n != 0).then(|| proportion(successes, n)),
        lower,
        upper,
        confidence: CONFIDENCE,
        method: "Wilson",
    }
}

/// Stable JSON shape for `adapt --json`.
#[derive(Serialize)]
struct AdaptReport<'a> {
    /// Schema identifier; always [`ADAPT_REPORT_SCHEMA`]. First field so it leads
    /// the emitted object.
    schema_version: &'static str,
    artifact: String,
    count: usize,
    findings: &'a [KeyFinding],
}

/// Stable JSON shape for `grade --json`.
#[derive(Serialize)]
struct GradeReport {
    /// Schema identifier; always [`GRADE_REPORT_SCHEMA`]. First field so it leads
    /// the emitted object.
    schema_version: &'static str,
    artifact: String,
    key: String,
    matched: usize,
    disputed: usize,
    missed: usize,
    /// Findings the schema-valid filter dropped before grading — malformed rows
    /// (empty id/category, out-of-range confidence, no content) that never
    /// entered the candidate set. Surfaced so a low match count is not misread:
    /// the review may have emitted invalid findings, not no findings.
    dropped_invalid: usize,
    /// Missed key rows that share a location with a disputed finding — correct
    /// locations the category-strict matcher could not confirm across the
    /// Cerberus/Daedalus vocabularies, recoverable by a downstream judge. Keeps
    /// the match rate from being read as a final recall.
    recoverable_misses: usize,
    match_rate: MatchRate,
}

/// Render the mapped answer-key rows as an aligned table.
fn print_adapt_table(artifact: &Path, rows: &[KeyFinding]) {
    println!("adapt {}", artifact.display());
    println!("{} mapped finding(s)\n", rows.len());
    if rows.is_empty() {
        println!("(no findings)");
        return;
    }

    let location: Vec<String> = rows.iter().map(location_label).collect();
    let severity: Vec<String> = rows.iter().map(|r| r.severity.clone()).collect();
    let category: Vec<String> = rows.iter().map(|r| r.category.clone()).collect();
    let description: Vec<String> = rows
        .iter()
        .map(|r| first_line_truncated(&r.description, DESC_WIDTH))
        .collect();

    let lw = column_width("LOCATION", &location);
    let sw = column_width("SEVERITY", &severity);
    let cw = column_width("CATEGORY", &category);

    println!(
        "{:<lw$}  {:<sw$}  {:<cw$}  DESCRIPTION",
        "LOCATION", "SEVERITY", "CATEGORY"
    );
    for i in 0..rows.len() {
        println!(
            "{:<lw$}  {:<sw$}  {:<cw$}  {}",
            location[i], severity[i], category[i], description[i]
        );
    }
}

/// Render the grade partition and the match-rate interval.
fn print_grade_summary(report: &GradeReport) {
    println!("crucible grade");
    println!("  artifact   {}", report.artifact);
    println!("  key        {}\n", report.key);
    println!("  matched    {}", report.matched);
    println!("  disputed   {}", report.disputed);
    println!("  missed     {}", report.missed);
    println!(
        "  dropped    {}  (schema-invalid findings)\n",
        report.dropped_invalid
    );

    let rate = &report.match_rate;
    match rate.point {
        None => println!("  match rate  n/a — no key rows to match"),
        Some(point) => println!(
            "  match rate  {:.1}%   {:.0}% CI [{:.1}%, {:.1}%]   (Wilson, matched/(matched+missed) = {}/{})",
            point * 100.0,
            rate.confidence * 100.0,
            rate.lower * 100.0,
            rate.upper * 100.0,
            rate.successes,
            rate.n,
        ),
    }

    if report.recoverable_misses > 0 {
        println!(
            "\n  note  {} missed key row(s) share a location with a disputed finding (category vocabulary mismatch); this recall is a category-strict pre-adjudication floor, not a final rate",
            report.recoverable_misses
        );
    }
}

/// Render the adjudication queue: the grade summary, the ordered items a judge
/// must rule on, and any labels already applied.
fn print_queue(artifact: &Path, key: &Path, queue: &JudgmentQueue) {
    let s = &queue.summary;
    println!("crucible adjudicate");
    println!("  artifact   {}", artifact.display());
    println!("  key        {}\n", key.display());
    println!("  matched    {}", s.matched);
    println!("  disputed   {}", s.disputed);
    println!(
        "  missed     {}  ({} recoverable)\n",
        s.missed, s.recoverable_misses
    );

    println!("  {} queue item(s)", queue.items.len());
    if queue.items.is_empty() {
        println!("  (nothing to adjudicate)");
    } else {
        print_queue_items(&queue.items);
    }

    if !queue.labels.is_empty() {
        println!("\n  {} label(s) applied", queue.labels.len());
        for l in &queue.labels {
            println!(
                "    {}  {:?}  in_scope={}",
                l.finding_id, l.verdict, l.disposition.in_scope
            );
        }
    }
}

/// Render the queue items as an aligned table: id, kind, location, category, and
/// a truncated description.
fn print_queue_items(items: &[JudgmentItem]) {
    let id: Vec<String> = items.iter().map(|i| i.finding_id.clone()).collect();
    let kind: Vec<String> = items
        .iter()
        .map(|i| {
            if i.is_recoverable() {
                "recoverable".to_string()
            } else {
                "dispute".to_string()
            }
        })
        .collect();
    let location: Vec<String> = items.iter().map(|i| location_label(&i.candidate)).collect();
    let category: Vec<String> = items.iter().map(|i| i.candidate.category.clone()).collect();
    let description: Vec<String> = items
        .iter()
        .map(|i| first_line_truncated(&i.candidate.description, DESC_WIDTH))
        .collect();

    let iw = column_width("ID", &id);
    let kw = column_width("KIND", &kind);
    let lw = column_width("LOCATION", &location);
    let cw = column_width("CATEGORY", &category);

    println!(
        "  {:<iw$}  {:<kw$}  {:<lw$}  {:<cw$}  DESCRIPTION",
        "ID", "KIND", "LOCATION", "CATEGORY"
    );
    for i in 0..items.len() {
        println!(
            "  {:<iw$}  {:<kw$}  {:<lw$}  {:<cw$}  {}",
            id[i], kind[i], location[i], category[i], description[i]
        );
    }
}

/// `file:line`, or a clear sentinel for the adapter's unanchored row.
fn location_label(row: &KeyFinding) -> String {
    if row.file.is_empty() {
        "(unanchored)".to_string()
    } else {
        format!("{}:{}", row.file, row.line)
    }
}

/// Widest of the header and every cell, for left-aligned columns.
///
/// Width is byte length, used as a display-width proxy. Exact here: every column
/// it measures — location (`file:line`), severity, category, id, kind — holds
/// ASCII, so bytes equal display columns. The one multi-byte glyph these tables
/// can emit (the `…` from [`first_line_truncated`]) lands only in the trailing,
/// unaligned DESCRIPTION column, which is never measured.
fn column_width(header: &str, cells: &[String]) -> usize {
    cells
        .iter()
        .map(String::len)
        .chain(std::iter::once(header.len()))
        .max()
        .unwrap_or(0)
}

/// First line of `s`, trimmed, truncated to `max` chars with an ellipsis.
fn first_line_truncated(s: &str, max: usize) -> String {
    let first = s.lines().next().unwrap_or("").trim();
    if first.chars().count() <= max {
        return first.to_string();
    }
    let take = max.saturating_sub(1).max(1);
    let head: String = first.chars().take(take).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crucible_core::Match;

    fn kf(file: &str, line: u32) -> KeyFinding {
        KeyFinding {
            file: file.to_string(),
            line,
            category: "security".to_string(),
            severity: "blocking".to_string(),
            description: "d".to_string(),
            source_id: None,
        }
    }

    #[test]
    fn match_rate_from_empty_grade_is_na_shaped() {
        // The n == 0 case (empty key) the CLI renders as "n/a": no key rows, so
        // the point estimate is `None` (serialized as null), distinguishable from
        // a true 0% by both `point` and `n`.
        let result = GradeResult {
            matched: Vec::new(),
            disputed: Vec::new(),
            missed: Vec::new(),
        };
        let rate = MatchRate::from_grade(&result);
        assert_eq!(rate.n, 0);
        assert_eq!(rate.successes, 0);
        assert_eq!(rate.point, None, "no key rows -> no point estimate");
        assert_eq!(rate.lower, 0.0);
        assert_eq!(rate.upper, 0.0);
    }

    #[test]
    fn match_rate_point_is_matched_over_matched_plus_missed() {
        // 1 matched + 1 missed -> recall 0.5 over n = 2; disputed does not enter
        // the denominator.
        let result = GradeResult {
            matched: vec![Match {
                candidate: kf("a.rs", 10),
                key: kf("a.rs", 10),
            }],
            disputed: vec![kf("z.rs", 99)],
            missed: vec![kf("b.rs", 20)],
        };
        let rate = MatchRate::from_grade(&result);
        assert_eq!(rate.successes, 1);
        assert_eq!(rate.n, 2);
        let point = rate.point.expect("a non-empty key has a point estimate");
        assert!((point - 0.5).abs() < 1e-9);
        assert!(rate.lower < point && point < rate.upper);
    }

    #[test]
    fn first_line_truncated_keeps_short_first_line() {
        assert_eq!(first_line_truncated("headline\n\nbody", 56), "headline");
    }

    #[test]
    fn first_line_truncated_adds_ellipsis_when_too_long() {
        let out = first_line_truncated("abcdefghij", 5);
        assert!(out.ends_with('…'), "truncated output ends with an ellipsis");
        assert_eq!(out.chars().count(), 5, "respects the max width");
    }

    #[test]
    fn location_label_marks_the_unanchored_sentinel() {
        assert_eq!(location_label(&kf("src/x.rs", 7)), "src/x.rs:7");
        assert_eq!(location_label(&kf("", 0)), "(unanchored)");
    }
}
