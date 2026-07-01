//! Crucible CLI — evaluate a Cerberus review run against a Daedalus answer key,
//! then queue what the deterministic floor cannot resolve for adjudication.
//!
//! Eight subcommands over the deterministic core:
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
//!   path is supplied, or runs the three built-in committed receipt checks when
//!   no spec is supplied. Every score carries a Wilson interval.
//! - `crucible adjudication-panel --queue <queue.json> --out <DIR>` renders an
//!   existing `crucible.judgment_queue.v1` artifact into a static phone-first
//!   `index.html` panel plus the copied `queue.json` model.
//! - `crucible mcp` serves the shared `crucible run` path over stdio MCP as the
//!   `crucible_run` tool, so agents and Threshold can invoke the same declared
//!   spec runner and get the same Wilson-scored run report.
//!
//! `--json` emits a stable serde object (`adapt`/`grade`/`adjudicate`); the
//! default is a human-readable table. `dashboard` instead writes files under
//! `--out` and prints a short receipt.
//!
//! **Exit codes** are stable so Cerberus/Daedalus can branch headlessly:
//! `0` success, `1` a load/parse failure (a bad artifact, key, or labels file),
//! `2` a usage error (bad arguments, surfaced by clap). `--help`/`--version` exit
//! `0`.

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

mod adjudication_panel;
mod dashboard_html;
mod eval_run;
mod mcp;
mod spec_run;

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
        #[arg(
            long,
            value_name = "DIR",
            default_value = "/Users/phaedrus/Development/daedalus/arenas"
        )]
        arenas: PathBuf,
        /// Runs tree (the trials) to read; defaults to the local Daedalus checkout.
        #[arg(
            long,
            value_name = "DIR",
            default_value = "/Users/phaedrus/Development/daedalus/runs"
        )]
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
    },
    /// Render a static phone-first adjudication panel from an existing
    /// `crucible.judgment_queue.v1` queue artifact.
    AdjudicationPanel {
        /// Path to a judgment queue JSON artifact.
        #[arg(long, value_name = "PATH")]
        queue: PathBuf,
        /// Output directory; `index.html` and a copied `queue.json` are written.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Serve Crucible's run surface as a stdio Model Context Protocol server.
    Mcp,
}

fn main() -> ExitCode {
    // clap owns usage errors (exit 2) and --help/--version (exit 0); everything
    // past parse is a load/parse path that fails with exit 1.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => err.exit(),
    };
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
        } => run_eval(spec.as_deref(), eval, out.as_deref(), json),
        Command::AdjudicationPanel { queue, out } => run_adjudication_panel(&queue, &out),
        Command::Mcp => mcp::run_stdio(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(EXIT_LOAD_ERROR)
        }
    }
}

/// `crucible run`: execute a declared spec when supplied, otherwise run built-in
/// eval receipts.
fn run_eval(
    spec: Option<&Path>,
    eval: eval_run::RunEval,
    out: Option<&Path>,
    json: bool,
) -> anyhow::Result<()> {
    let report = if let Some(spec_path) = spec {
        if eval != eval_run::RunEval::All {
            anyhow::bail!(
                "--eval selects built-in receipts and cannot be combined with a spec path"
            );
        }
        spec_run::run(spec_path, out)?
    } else {
        let out = out.with_context(|| "built-in receipt runs require --out <DIR>")?;
        eval_run::run(eval, out)?
    };
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
    }
    Ok(())
}

/// `crucible adjudication-panel`: render a phone-first static HTML panel from an
/// existing judgment queue.
fn run_adjudication_panel(queue: &Path, out: &Path) -> anyhow::Result<()> {
    let receipt = adjudication_panel::write_panel(queue, out)?;
    println!("crucible adjudication-panel");
    println!("  queue    {}", queue.display());
    println!("  items    {}", receipt.items);
    println!("  labels   {}", receipt.labels);
    println!("  wrote    {}", receipt.html_path.display());
    println!("  wrote    {}", receipt.queue_path.display());
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
    let (candidates, dropped_invalid) = candidate_rows(artifact)?;
    let key_rows = load_key_rows(key_path)?;

    let result = grade(&candidates, &key_rows);
    let match_rate = MatchRate::from_grade(&result);
    let recoverable = recoverable_misses(&result);

    if json {
        let report = GradeReport {
            schema_version: GRADE_REPORT_SCHEMA,
            artifact: artifact.display().to_string(),
            key: key_path.display().to_string(),
            matched: result.matched.len(),
            disputed: result.disputed.len(),
            missed: result.missed.len(),
            dropped_invalid,
            recoverable_misses: recoverable,
            match_rate,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_grade_summary(
            artifact,
            key_path,
            &result,
            dropped_invalid,
            &match_rate,
            recoverable,
        );
    }
    Ok(())
}

/// `crucible adjudicate`: grade, build the queue, optionally apply labels, emit.
fn run_adjudicate(
    artifact: &Path,
    key_path: &Path,
    apply: Option<&Path>,
    json: bool,
) -> anyhow::Result<()> {
    let (candidates, _dropped) = candidate_rows(artifact)?;
    let key_rows = load_key_rows(key_path)?;
    let result = grade(&candidates, &key_rows);
    let mut queue = build_queue(&result);

    if let Some(apply_path) = apply {
        queue.labels = mint_labels(&queue, &load_decisions(apply_path)?)?;
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&queue)?);
    } else {
        print_queue(artifact, key_path, &queue);
    }
    Ok(())
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
    println!("crucible export");
    println!("  arena         {arena}");
    println!("  task          {task}");
    println!(
        "  adjudications {}  ({accepts} accept, {} out-of-scope)",
        adjudications.len(),
        adjudications.len() - accepts,
    );
    println!("  wrote         {}", md_path.display());
    if let Some(p) = key_path {
        println!("  oracle key    {}", p.display());
    }
    if let Some(p) = expected_path {
        println!("  scorer key    {}", p.display());
    }
    Ok(())
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
fn print_grade_summary(
    artifact: &Path,
    key: &Path,
    result: &GradeResult,
    dropped_invalid: usize,
    rate: &MatchRate,
    recoverable: usize,
) {
    println!("crucible grade");
    println!("  artifact   {}", artifact.display());
    println!("  key        {}\n", key.display());
    println!("  matched    {}", result.matched.len());
    println!("  disputed   {}", result.disputed.len());
    println!("  missed     {}", result.missed.len());
    println!("  dropped    {dropped_invalid}  (schema-invalid findings)\n");

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

    if recoverable > 0 {
        println!(
            "\n  note  {recoverable} missed key row(s) share a location with a disputed finding (category vocabulary mismatch); this recall is a category-strict pre-adjudication floor, not a final rate"
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
