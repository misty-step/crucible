//! `crucible author`: assemble a valid `EvalSpec` without hand-writing JSON
//! (backlog/Powder crucible-942).
//!
//! Two ways in: non-interactive flags (`--task-family`, `--runner-kind`,
//! `--prompt-*`/`--key-recall-*`, ...) for a scriptable/cold-agent path, or
//! `--interactive` for a guided stdin/stdout prompt flow (plain
//! `BufRead::read_line`, no TUI dependency). Both converge on the same
//! [`AuthorInputs`] -> [`EvalSpec`] assembly, then the same save gate: the
//! assembled spec is written to a scratch file beside the real output path,
//! run through [`crate::validate::validate`] — the exact function `crucible
//! validate` calls, never forked — and only renamed into place when
//! `report.valid`. An invalid assembly is refused with the same
//! `{valid, runnable, errors, warnings}` shape `crucible validate` prints,
//! and leaves no file at the output path.
//!
//! Covers the two runner kinds this pass commits to: `key_recall` (over a
//! Daedalus `trials.jsonl` corpus — the shape the flagship
//! `pr-review-key-recall-v0.json` uses) and `prompt_benchmark` (one authored
//! task per invocation; re-run `author` for additional tasks, or hand-edit
//! the `tasks` array afterward — this stays a single-task wedge on purpose).
//! `agentic_judge` authoring is a documented follow-up (`backlog.d/`): its
//! judge-gaming canary and calibration-probe shape need a richer prompt flow
//! than this pass's flag/stdin surface covers well.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Args, ValueEnum};
use serde::Serialize;

use crucible_core::{
    AggregationMethod, CorpusSpec, EvalSpec, Grader, GraderKind, GraderManifest, ModelProvider,
    PromptBenchmarkTask, PromptExpectation, PromptModelConfig, RunnerKind, RunnerSpec,
    UncertaintyRule, EVAL_SPEC_SCHEMA,
};

use crate::spec_run::required_grader_kind;
use crate::validate::{self, ValidationReport};

/// Schema identifier for `crucible author --json`'s report.
pub const AUTHOR_REPORT_SCHEMA: &str = "crucible.author_report.v1";

/// The runner kinds `crucible author` can assemble this pass. `snake_case`
/// on the command line to match the wire vocabulary every other
/// `RunnerKind`/`GraderKind`/`PromptExpectation` value already uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum AuthorRunnerKind {
    KeyRecall,
    PromptBenchmark,
}

/// The deterministic rubric kinds `crucible author` can assemble for a
/// `prompt_benchmark` task's single-task convenience flags. `snake_case` on
/// the command line to match [`PromptExpectation`]'s own wire vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum AuthorExpectationKind {
    Exact,
    Contains,
    CaseInsensitiveContains,
    Regex,
    StrictJson,
}

/// Flags for `crucible author`. Every field is optional at the clap layer —
/// [`AuthorInputs::from_flags`] enforces the real per-runner-kind
/// requirements so a missing flag reports a clear error naming exactly what
/// was needed, rather than clap's generic "required" message picking one
/// fixed shape for every runner kind.
#[derive(Debug, Args)]
pub struct AuthorArgs {
    /// Guided stdin/stdout prompt flow instead of flags.
    #[arg(long)]
    pub interactive: bool,

    /// Output path for the assembled spec JSON. Defaults to
    /// `evals/<id-or-task-slug>.json`.
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,

    /// Overwrite an existing file at the output path. Without this, an
    /// existing file at `--out` refuses the write instead of silently
    /// clobbering a hand-authored (or previously authored) spec.
    #[arg(long)]
    pub force: bool,

    /// Emit the `{valid, runnable, errors, warnings}` report and the
    /// resolved output path as stable JSON instead of a readable summary.
    #[arg(long)]
    pub json: bool,

    /// Stable eval id, e.g. `my-eval-v0`. Defaults to the output file stem.
    #[arg(long)]
    pub id: Option<String>,

    /// The task family this eval measures, e.g. `code-review`. Required
    /// unless `--interactive`.
    #[arg(long = "task-family", value_name = "TASK")]
    pub task_family: Option<String>,

    /// Free-form description of the inputs this eval consumes.
    #[arg(long)]
    pub inputs: Option<String>,

    /// Free-form description of the outputs this eval scores.
    #[arg(long)]
    pub outputs: Option<String>,

    /// The decision this eval informs, in one sentence.
    #[arg(long)]
    pub decision: Option<String>,

    /// A named baseline config to compare against. Repeatable.
    #[arg(long = "baseline", value_name = "NAME")]
    pub baselines: Vec<String>,

    /// One grader in the mix, `<id>:<kind>` where kind is
    /// `deterministic|agentic|human`. Repeatable. When none are given, one
    /// canonical grader of the chosen runner's required kind is added
    /// automatically so the assembled spec is runnable out of the box.
    #[arg(long = "grader", value_name = "ID:KIND")]
    pub graders: Vec<String>,

    /// Which runner this spec declares. Required unless `--interactive`.
    #[arg(long = "runner-kind", value_enum)]
    pub runner_kind: Option<AuthorRunnerKind>,

    /// `key_recall`: Daedalus arena directory, absolute or relative to the
    /// eventual spec file.
    #[arg(long = "key-recall-arena-dir", value_name = "PATH")]
    pub key_recall_arena_dir: Option<String>,
    /// `key_recall`: Daedalus `trials.jsonl` file, absolute or relative to
    /// the eventual spec file.
    #[arg(long = "key-recall-trials-jsonl", value_name = "PATH")]
    pub key_recall_trials_jsonl: Option<String>,
    /// `key_recall`: candidate id to select from the trials file.
    #[arg(long = "key-recall-candidate-id", value_name = "ID")]
    pub key_recall_candidate_id: Option<String>,
    /// `key_recall`: a task id to select. Repeatable; omit entirely to
    /// select every trial for the candidate.
    #[arg(long = "key-recall-task", value_name = "TASK_ID")]
    pub key_recall_tasks: Vec<String>,

    /// `prompt_benchmark`: OpenRouter model slug, e.g. `openai/gpt-4o-mini`.
    #[arg(long = "prompt-model", value_name = "SLUG")]
    pub prompt_model: Option<String>,
    /// `prompt_benchmark`: system prompt shared by the authored task.
    #[arg(long = "prompt-system-prompt", value_name = "TEXT")]
    pub prompt_system_prompt: Option<String>,
    /// `prompt_benchmark`: env var carrying the provider credential.
    /// Defaults to `OPENROUTER_API_KEY`.
    #[arg(long = "prompt-credential-env", value_name = "ENV")]
    pub prompt_credential_env: Option<String>,
    /// `prompt_benchmark`: optional output cap for the model call.
    #[arg(long = "prompt-max-output-units", value_name = "N")]
    pub prompt_max_output_units: Option<u32>,
    /// `prompt_benchmark`: optional integer temperature.
    #[arg(long = "prompt-temperature", value_name = "N")]
    pub prompt_temperature: Option<u32>,
    /// `prompt_benchmark`: the authored task's stable id.
    #[arg(long = "prompt-task-id", value_name = "ID")]
    pub prompt_task_id: Option<String>,
    /// `prompt_benchmark`: the authored task's user prompt.
    #[arg(long = "prompt-task-prompt", value_name = "TEXT")]
    pub prompt_task_prompt: Option<String>,
    /// `prompt_benchmark`: optional reporting class, e.g. `format_adherence`.
    #[arg(long = "prompt-task-class", value_name = "CLASS")]
    pub prompt_task_class: Option<String>,
    /// `prompt_benchmark`: optional prompt-context file, absolute or
    /// relative to the eventual spec file.
    #[arg(long = "prompt-task-context-file", value_name = "PATH")]
    pub prompt_task_context_file: Option<String>,
    /// `prompt_benchmark`: the task's deterministic rubric kind.
    #[arg(long = "prompt-expectation-kind", value_enum)]
    pub prompt_expectation_kind: Option<AuthorExpectationKind>,
    /// `prompt_benchmark`: the rubric value — exact/contains text, a regex
    /// pattern, or (for `strict_json`) a literal JSON value.
    #[arg(long = "prompt-expectation-value", value_name = "TEXT")]
    pub prompt_expectation_value: Option<String>,
}

/// `crucible author`: build, validate, and (if valid) save an [`EvalSpec`].
pub fn run(args: AuthorArgs) -> anyhow::Result<()> {
    let inputs = if args.interactive {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut reader = stdin.lock();
        let mut writer = stdout.lock();
        AuthorInputs::from_interactive(&mut reader, &mut writer)?
    } else {
        AuthorInputs::from_flags(&args)?
    };

    let report = assemble_and_write(inputs, args.out.as_deref(), args.force)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_author_report(Path::new(&report.out), report.written, &report.validate);
    }

    if !report.written {
        anyhow::bail!(
            "assembled spec failed validation; refusing to write {} (see errors above)",
            report.out
        );
    }
    Ok(())
}

/// The non-interactive half of `crucible author`, factored out so callers
/// that cannot drive a stdin prompt flow — namely MCP's `crucible_author`
/// tool — can assemble, validate, and (if valid) save an [`EvalSpec`] from
/// flags alone and get back a structured [`AuthorReport`] instead of parsing
/// CLI stdout.
pub fn author_from_flags(args: &AuthorArgs) -> anyhow::Result<AuthorReport> {
    let inputs = AuthorInputs::from_flags(args)?;
    assemble_and_write(inputs, args.out.as_deref(), args.force)
}

/// Shared tail of both authoring paths: turn resolved [`AuthorInputs`] into
/// an [`EvalSpec`], resolve the output path, refuse an unintended overwrite,
/// and run the same validate-then-save gate `crucible validate` performs.
fn assemble_and_write(
    inputs: AuthorInputs,
    out: Option<&Path>,
    force: bool,
) -> anyhow::Result<AuthorReport> {
    let spec = inputs.into_eval_spec();
    let out_path = resolve_out_path(out, &spec);

    if out_path.exists() && !force {
        anyhow::bail!(
            "refusing to overwrite existing spec at {} (pass --force to overwrite)",
            out_path.display()
        );
    }

    let (report, written) = validate_and_maybe_write(&spec, &out_path)?;
    Ok(AuthorReport {
        schema_version: AUTHOR_REPORT_SCHEMA,
        out: out_path.display().to_string(),
        written,
        validate: report,
    })
}

/// Stable JSON shape for `crucible author --json`, and the structured
/// content MCP's `crucible_author` tool returns.
#[derive(Debug, Serialize)]
pub struct AuthorReport {
    pub schema_version: &'static str,
    pub out: String,
    pub written: bool,
    pub validate: ValidationReport,
}

fn print_author_report(out_path: &Path, written: bool, report: &ValidationReport) {
    println!("crucible author");
    println!("  out       {}", out_path.display());
    println!("  valid     {}", report.valid);
    println!("  runnable  {}", report.runnable);
    for error in &report.errors {
        println!("  ERROR     {}: {}", error.field, error.message);
    }
    for warning in &report.warnings {
        println!("  warning   {}: {}", warning.field, warning.message);
    }
    if written {
        println!("  wrote     {}", out_path.display());
    } else {
        println!("  refused   spec failed validation; nothing written");
    }
}

/// Write the assembled spec to a scratch file beside `out_path`, validate it
/// through the exact `crucible validate` path, and rename it into place iff
/// valid. Returns the report (with `spec` rewritten to `out_path` so the
/// printed report never leaks the scratch filename) and whether the file was
/// actually written.
fn validate_and_maybe_write(
    spec: &EvalSpec,
    out_path: &Path,
) -> anyhow::Result<(ValidationReport, bool)> {
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
    }
    let tmp_path = temp_sibling_path(out_path)?;
    let json = serde_json::to_string_pretty(spec).context("serializing assembled spec")?;
    std::fs::write(&tmp_path, format!("{json}\n"))
        .with_context(|| format!("writing scratch spec {}", tmp_path.display()))?;

    let mut report = match validate::validate(&tmp_path) {
        Ok(report) => report,
        Err(err) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(err);
        }
    };
    // The scratch path only exists to let `validate::validate` run unforked;
    // the report the operator/agent sees should name the real destination.
    report.spec = out_path.display().to_string();

    if report.valid {
        if std::fs::rename(&tmp_path, out_path).is_err() {
            // Cross-device out paths (rare, e.g. --out on a different mount
            // than evals/) can't rename; fall back to copy + remove.
            std::fs::copy(&tmp_path, out_path)
                .with_context(|| format!("writing assembled spec to {}", out_path.display()))?;
            let _ = std::fs::remove_file(&tmp_path);
        }
        Ok((report, true))
    } else {
        let _ = std::fs::remove_file(&tmp_path);
        Ok((report, false))
    }
}

fn temp_sibling_path(out_path: &Path) -> anyhow::Result<PathBuf> {
    let parent = out_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = out_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("spec.json");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    Ok(parent.join(format!(
        ".crucible-author-{}-{nonce}-{name}",
        std::process::id()
    )))
}

fn resolve_out_path(out: Option<&Path>, spec: &EvalSpec) -> PathBuf {
    if let Some(out) = out {
        return out.to_path_buf();
    }
    let slug = if !spec.id.trim().is_empty() {
        slugify(&spec.id)
    } else {
        slugify(&spec.task)
    };
    Path::new("evals").join(format!("{slug}.json"))
}

/// Lowercase-alnum-and-dash slug, used only for a friendly default `--out`
/// filename when one isn't given — never for anything Crucible reads back
/// structurally.
fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash && !out.is_empty() {
            out.push('-');
            last_was_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "eval".to_string()
    } else {
        out
    }
}

/// One resolved runner declaration: which kind, and its corpus.
#[derive(Debug)]
struct ResolvedRunner {
    kind: RunnerKind,
    corpus: CorpusSpec,
}

/// Everything needed to assemble an [`EvalSpec`], gathered from either flags
/// or the interactive prompt flow.
#[derive(Debug)]
struct AuthorInputs {
    id: Option<String>,
    task: String,
    inputs: String,
    outputs: String,
    decision: String,
    baselines: Vec<String>,
    graders: Vec<Grader>,
    runner: ResolvedRunner,
}

impl AuthorInputs {
    /// Resolve from `--task-family`/`--runner-kind`/etc, erroring with a
    /// message naming exactly which flag is missing for the chosen runner
    /// kind — before anything touches the filesystem.
    fn from_flags(args: &AuthorArgs) -> anyhow::Result<Self> {
        let task = required_str(&args.task_family, "--task-family")
            .context("--task-family is required in non-interactive mode (or pass --interactive)")?;
        let runner_kind = args.runner_kind.context(
            "--runner-kind <key_recall|prompt_benchmark> is required in non-interactive mode (or pass --interactive)",
        )?;
        let graders = parse_graders(&args.graders)?;
        let runner = match runner_kind {
            AuthorRunnerKind::KeyRecall => key_recall_from_flags(args)?,
            AuthorRunnerKind::PromptBenchmark => prompt_benchmark_from_flags(args)?,
        };

        Ok(Self {
            id: non_empty(&args.id),
            task,
            inputs: args.inputs.clone().unwrap_or_default(),
            outputs: args.outputs.clone().unwrap_or_default(),
            decision: args.decision.clone().unwrap_or_default(),
            baselines: args.baselines.clone(),
            graders,
            runner,
        })
    }

    /// Guided stdin/stdout flow: the same fields `from_flags` needs, asked
    /// one at a time with `read_line`. Generic over the reader/writer so a
    /// test can drive it with an in-memory `Cursor` instead of a real
    /// terminal.
    fn from_interactive<R: BufRead, W: Write>(
        reader: &mut R,
        writer: &mut W,
    ) -> anyhow::Result<Self> {
        writeln!(writer, "crucible author --interactive")?;
        writeln!(
            writer,
            "Guided eval-spec authoring. Blank answers accept the default shown, or leave a free-form field empty."
        )?;

        let id = prompt_optional(reader, writer, "Eval id (blank = derived from --out)")?;
        let task = prompt_required(reader, writer, "Task family (e.g. code-review)")?;
        let inputs = prompt_optional(reader, writer, "Inputs description")?.unwrap_or_default();
        let outputs = prompt_optional(reader, writer, "Outputs description")?.unwrap_or_default();
        let decision =
            prompt_optional(reader, writer, "Decision this eval informs")?.unwrap_or_default();
        let baselines = prompt_csv(reader, writer, "Baselines (comma-separated, blank = none)")?;

        let runner_kind = prompt_runner_kind(reader, writer)?;
        let runner = match runner_kind {
            AuthorRunnerKind::KeyRecall => key_recall_from_interactive(reader, writer)?,
            AuthorRunnerKind::PromptBenchmark => prompt_benchmark_from_interactive(reader, writer)?,
        };

        let grader_line = prompt_csv(
            reader,
            writer,
            "Additional graders beyond the automatic default (comma-separated id:kind, blank = none)",
        )?;
        let graders = parse_graders(&grader_line)?;

        Ok(Self {
            id,
            task,
            inputs,
            outputs,
            decision,
            baselines,
            graders,
            runner,
        })
    }

    /// Assemble the final [`EvalSpec`]. When no grader was named, one
    /// canonical grader of the runner's required kind is added so the
    /// resulting spec is runnable, not merely definition-only. An explicitly
    /// named grader set that still lacks the required kind is left as-is —
    /// `crucible validate` (via the save gate) reports that clearly instead
    /// of this function silently rewriting the operator's declared mix.
    fn into_eval_spec(self) -> EvalSpec {
        let required_kind = required_grader_kind(self.runner.kind);
        let mut graders = self.graders;
        if graders.is_empty() {
            graders.push(default_grader_for(required_kind));
        }

        EvalSpec {
            schema_version: EVAL_SPEC_SCHEMA.to_string(),
            id: self.id.unwrap_or_default(),
            task: self.task,
            inputs: self.inputs,
            outputs: self.outputs,
            fixtures: Vec::new(),
            graders: GraderManifest { graders },
            baselines: self.baselines,
            aggregation: AggregationMethod::Proportion,
            uncertainty: UncertaintyRule::default(),
            decision: self.decision,
            runner: Some(RunnerSpec {
                kind: self.runner.kind,
                corpus: self.runner.corpus,
            }),
        }
    }
}

fn default_grader_for(required: GraderKind) -> Grader {
    let id = match required {
        GraderKind::Deterministic => "deterministic_rubric",
        GraderKind::Agentic => "model_judge",
        GraderKind::Human => "operator",
    };
    Grader {
        id: id.to_string(),
        kind: required,
    }
}

fn key_recall_from_flags(args: &AuthorArgs) -> anyhow::Result<ResolvedRunner> {
    let arena_dir = required_str(&args.key_recall_arena_dir, "--key-recall-arena-dir")
        .context("--runner-kind key_recall requires --key-recall-arena-dir")?;
    let trials_jsonl = required_str(&args.key_recall_trials_jsonl, "--key-recall-trials-jsonl")
        .context("--runner-kind key_recall requires --key-recall-trials-jsonl")?;
    let candidate_id = required_str(&args.key_recall_candidate_id, "--key-recall-candidate-id")
        .context("--runner-kind key_recall requires --key-recall-candidate-id")?;
    Ok(ResolvedRunner {
        kind: RunnerKind::KeyRecall,
        corpus: CorpusSpec::DaedalusTrials {
            arena_dir,
            trials_jsonl,
            candidate_id,
            tasks: args.key_recall_tasks.clone(),
        },
    })
}

fn key_recall_from_interactive<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
) -> anyhow::Result<ResolvedRunner> {
    let arena_dir = prompt_required(
        reader,
        writer,
        "Daedalus arena_dir (absolute or relative to the spec file)",
    )?;
    let trials_jsonl = prompt_required(
        reader,
        writer,
        "Daedalus trials_jsonl (absolute or relative to the spec file)",
    )?;
    let candidate_id = prompt_required(reader, writer, "Candidate id")?;
    let tasks = prompt_csv(
        reader,
        writer,
        "Task ids to select (comma-separated, blank = every trial for the candidate)",
    )?;
    Ok(ResolvedRunner {
        kind: RunnerKind::KeyRecall,
        corpus: CorpusSpec::DaedalusTrials {
            arena_dir,
            trials_jsonl,
            candidate_id,
            tasks,
        },
    })
}

fn prompt_benchmark_from_flags(args: &AuthorArgs) -> anyhow::Result<ResolvedRunner> {
    let model = required_str(&args.prompt_model, "--prompt-model")
        .context("--runner-kind prompt_benchmark requires --prompt-model")?;
    let system_prompt = required_str(&args.prompt_system_prompt, "--prompt-system-prompt")
        .context("--runner-kind prompt_benchmark requires --prompt-system-prompt")?;
    let credential_env =
        non_empty(&args.prompt_credential_env).unwrap_or_else(|| "OPENROUTER_API_KEY".to_string());
    let task = prompt_task_from_flags(args)?;

    Ok(ResolvedRunner {
        kind: RunnerKind::PromptBenchmark,
        corpus: CorpusSpec::PromptBenchmark {
            config: PromptModelConfig {
                provider: ModelProvider::OpenRouter,
                model,
                system_prompt,
                credential_env,
                max_output_units: args.prompt_max_output_units,
                temperature: args.prompt_temperature,
            },
            tasks: vec![task],
        },
    })
}

fn prompt_task_from_flags(args: &AuthorArgs) -> anyhow::Result<PromptBenchmarkTask> {
    let task_id = required_str(&args.prompt_task_id, "--prompt-task-id")
        .context("--runner-kind prompt_benchmark requires --prompt-task-id")?;
    let prompt = required_str(&args.prompt_task_prompt, "--prompt-task-prompt")
        .context("--runner-kind prompt_benchmark requires --prompt-task-prompt")?;
    let kind = args.prompt_expectation_kind.context(
        "--runner-kind prompt_benchmark requires --prompt-expectation-kind <exact|contains|case_insensitive_contains|regex|strict_json>",
    )?;
    let value = required_str(&args.prompt_expectation_value, "--prompt-expectation-value")
        .context("--runner-kind prompt_benchmark requires --prompt-expectation-value")?;
    let expectation = build_expectation(kind, &value)?;
    Ok(PromptBenchmarkTask {
        task_id,
        class: non_empty(&args.prompt_task_class),
        context_file: non_empty(&args.prompt_task_context_file),
        prompt,
        expectation,
    })
}

fn prompt_benchmark_from_interactive<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
) -> anyhow::Result<ResolvedRunner> {
    let model = prompt_required(reader, writer, "Model slug (e.g. openai/gpt-4o-mini)")?;
    let system_prompt = prompt_required(reader, writer, "System prompt")?;
    let credential_env =
        prompt_with_default(reader, writer, "Credential env var", "OPENROUTER_API_KEY")?;
    let max_output_units = prompt_optional_u32(reader, writer, "Max output tokens (blank = none)")?;
    let temperature = prompt_optional_u32(reader, writer, "Temperature (blank = none)")?;

    let task_id = prompt_required(reader, writer, "Task id")?;
    let prompt_text = prompt_required(reader, writer, "Task prompt")?;
    let class = prompt_optional(reader, writer, "Task class (blank = none)")?;
    let kind = prompt_expectation_kind(reader, writer)?;
    let value = prompt_required(
        reader,
        writer,
        "Expectation value (for strict_json, a literal JSON value)",
    )?;
    let expectation = build_expectation(kind, &value)?;

    Ok(ResolvedRunner {
        kind: RunnerKind::PromptBenchmark,
        corpus: CorpusSpec::PromptBenchmark {
            config: PromptModelConfig {
                provider: ModelProvider::OpenRouter,
                model,
                system_prompt,
                credential_env,
                max_output_units,
                temperature,
            },
            tasks: vec![PromptBenchmarkTask {
                task_id,
                class,
                context_file: None,
                prompt: prompt_text,
                expectation,
            }],
        },
    })
}

fn build_expectation(
    kind: AuthorExpectationKind,
    value: &str,
) -> anyhow::Result<PromptExpectation> {
    Ok(match kind {
        AuthorExpectationKind::Exact => PromptExpectation::Exact {
            value: value.to_string(),
        },
        AuthorExpectationKind::Contains => PromptExpectation::Contains {
            value: value.to_string(),
        },
        AuthorExpectationKind::CaseInsensitiveContains => {
            PromptExpectation::CaseInsensitiveContains {
                value: value.to_string(),
            }
        }
        AuthorExpectationKind::Regex => PromptExpectation::Regex {
            pattern: value.to_string(),
        },
        AuthorExpectationKind::StrictJson => {
            let parsed: serde_json::Value = serde_json::from_str(value).with_context(|| {
                format!(
                    "--prompt-expectation-value must be valid JSON for strict_json, got {value:?}"
                )
            })?;
            PromptExpectation::StrictJson { value: parsed }
        }
    })
}

fn parse_graders(raw: &[String]) -> anyhow::Result<Vec<Grader>> {
    raw.iter().map(|entry| parse_grader(entry)).collect()
}

fn parse_grader(entry: &str) -> anyhow::Result<Grader> {
    let (id, kind) = entry
        .split_once(':')
        .with_context(|| format!("--grader {entry:?} must be `<id>:<kind>`"))?;
    let id = id.trim();
    let kind = kind.trim();
    if id.is_empty() {
        anyhow::bail!("--grader {entry:?} has an empty id");
    }
    let kind = match kind {
        "deterministic" => GraderKind::Deterministic,
        "agentic" => GraderKind::Agentic,
        "human" => GraderKind::Human,
        other => anyhow::bail!(
            "--grader {entry:?} has an unknown kind {other:?}; expected deterministic|agentic|human"
        ),
    };
    Ok(Grader {
        id: id.to_string(),
        kind,
    })
}

fn required_str(value: &Option<String>, flag: &str) -> anyhow::Result<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .with_context(|| format!("{flag} is required"))
}

fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn prompt_line<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> anyhow::Result<String> {
    write!(writer, "{label}: ")?;
    writer.flush()?;
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .context("reading interactive stdin")?;
    if n == 0 {
        anyhow::bail!("interactive input ended before {label:?} was answered");
    }
    Ok(line.trim().to_string())
}

fn prompt_required<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> anyhow::Result<String> {
    loop {
        let line = prompt_line(reader, writer, label)?;
        if !line.is_empty() {
            return Ok(line);
        }
        writeln!(writer, "  (required, try again)")?;
    }
}

fn prompt_optional<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> anyhow::Result<Option<String>> {
    let line = prompt_line(reader, writer, label)?;
    Ok((!line.is_empty()).then_some(line))
}

fn prompt_with_default<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
    default: &str,
) -> anyhow::Result<String> {
    let line = prompt_line(reader, writer, &format!("{label} [{default}]"))?;
    Ok(if line.is_empty() {
        default.to_string()
    } else {
        line
    })
}

fn prompt_csv<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> anyhow::Result<Vec<String>> {
    let line = prompt_line(reader, writer, label)?;
    Ok(line
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

fn prompt_optional_u32<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    label: &str,
) -> anyhow::Result<Option<u32>> {
    let line = prompt_line(reader, writer, label)?;
    if line.is_empty() {
        return Ok(None);
    }
    line.parse::<u32>()
        .map(Some)
        .with_context(|| format!("{label} must be a whole number, got {line:?}"))
}

fn prompt_runner_kind<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
) -> anyhow::Result<AuthorRunnerKind> {
    loop {
        let line = prompt_required(reader, writer, "Runner kind [key_recall/prompt_benchmark]")?;
        match line.as_str() {
            "key_recall" => return Ok(AuthorRunnerKind::KeyRecall),
            "prompt_benchmark" => return Ok(AuthorRunnerKind::PromptBenchmark),
            other => writeln!(
                writer,
                "  unknown runner kind {other:?}; expected key_recall or prompt_benchmark"
            )?,
        }
    }
}

fn prompt_expectation_kind<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
) -> anyhow::Result<AuthorExpectationKind> {
    loop {
        let line = prompt_required(
            reader,
            writer,
            "Expectation kind [exact/contains/case_insensitive_contains/regex/strict_json]",
        )?;
        match line.as_str() {
            "exact" => return Ok(AuthorExpectationKind::Exact),
            "contains" => return Ok(AuthorExpectationKind::Contains),
            "case_insensitive_contains" => {
                return Ok(AuthorExpectationKind::CaseInsensitiveContains)
            }
            "regex" => return Ok(AuthorExpectationKind::Regex),
            "strict_json" => return Ok(AuthorExpectationKind::StrictJson),
            other => writeln!(writer, "  unknown expectation kind {other:?}")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crucible-author-unit-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn base_args() -> AuthorArgs {
        AuthorArgs {
            interactive: false,
            out: None,
            force: false,
            json: false,
            id: None,
            task_family: None,
            inputs: None,
            outputs: None,
            decision: None,
            baselines: Vec::new(),
            graders: Vec::new(),
            runner_kind: None,
            key_recall_arena_dir: None,
            key_recall_trials_jsonl: None,
            key_recall_candidate_id: None,
            key_recall_tasks: Vec::new(),
            prompt_model: None,
            prompt_system_prompt: None,
            prompt_credential_env: None,
            prompt_max_output_units: None,
            prompt_temperature: None,
            prompt_task_id: None,
            prompt_task_prompt: None,
            prompt_task_class: None,
            prompt_task_context_file: None,
            prompt_expectation_kind: None,
            prompt_expectation_value: None,
        }
    }

    #[test]
    fn from_flags_requires_task_family() {
        let args = base_args();
        let err = AuthorInputs::from_flags(&args).unwrap_err();
        assert!(err.to_string().contains("--task-family"), "{err}");
    }

    #[test]
    fn from_flags_requires_runner_kind() {
        let mut args = base_args();
        args.task_family = Some("code-review".to_string());
        let err = AuthorInputs::from_flags(&args).unwrap_err();
        assert!(err.to_string().contains("--runner-kind"), "{err}");
    }

    #[test]
    fn from_flags_prompt_benchmark_requires_model_and_task_fields() {
        let mut args = base_args();
        args.task_family = Some("prompt-smoke".to_string());
        args.runner_kind = Some(AuthorRunnerKind::PromptBenchmark);
        let err = AuthorInputs::from_flags(&args).unwrap_err();
        assert!(err.to_string().contains("--prompt-model"), "{err}");
    }

    #[test]
    fn from_flags_key_recall_requires_corpus_fields() {
        let mut args = base_args();
        args.task_family = Some("pr-review".to_string());
        args.runner_kind = Some(AuthorRunnerKind::KeyRecall);
        let err = AuthorInputs::from_flags(&args).unwrap_err();
        assert!(err.to_string().contains("--key-recall-arena-dir"), "{err}");
    }

    #[test]
    fn from_flags_assembles_a_runnable_prompt_benchmark_spec_with_default_grader() {
        let mut args = base_args();
        args.task_family = Some("prompt-smoke".to_string());
        args.runner_kind = Some(AuthorRunnerKind::PromptBenchmark);
        args.prompt_model = Some("openrouter/auto".to_string());
        args.prompt_system_prompt = Some("Answer exactly.".to_string());
        args.prompt_task_id = Some("marker-echo".to_string());
        args.prompt_task_prompt = Some("Reply with crucible-smoke".to_string());
        args.prompt_expectation_kind = Some(AuthorExpectationKind::Contains);
        args.prompt_expectation_value = Some("crucible-smoke".to_string());

        let inputs = AuthorInputs::from_flags(&args).unwrap();
        let spec = inputs.into_eval_spec();
        assert_eq!(spec.task, "prompt-smoke");
        assert_eq!(spec.graders.graders.len(), 1);
        assert_eq!(spec.graders.graders[0].kind, GraderKind::Deterministic);
        assert!(spec.runner.is_some());
    }

    #[test]
    fn into_eval_spec_does_not_override_an_explicit_grader_mix() {
        let mut args = base_args();
        args.task_family = Some("prompt-smoke".to_string());
        args.runner_kind = Some(AuthorRunnerKind::PromptBenchmark);
        args.prompt_model = Some("openrouter/auto".to_string());
        args.prompt_system_prompt = Some("Answer exactly.".to_string());
        args.prompt_task_id = Some("marker-echo".to_string());
        args.prompt_task_prompt = Some("Reply with crucible-smoke".to_string());
        args.prompt_expectation_kind = Some(AuthorExpectationKind::Contains);
        args.prompt_expectation_value = Some("crucible-smoke".to_string());
        // Deliberately the wrong kind for prompt_benchmark (needs
        // deterministic) — this must survive assembly unchanged so the save
        // gate (crucible validate) is what refuses it, not a silent rewrite.
        args.graders = vec!["operator:human".to_string()];

        let inputs = AuthorInputs::from_flags(&args).unwrap();
        let spec = inputs.into_eval_spec();
        assert_eq!(spec.graders.graders.len(), 1);
        assert_eq!(spec.graders.graders[0].id, "operator");
        assert_eq!(spec.graders.graders[0].kind, GraderKind::Human);
    }

    #[test]
    fn parse_grader_rejects_missing_colon() {
        let err = parse_grader("just_an_id").unwrap_err();
        assert!(err.to_string().contains("<id>:<kind>"), "{err}");
    }

    #[test]
    fn parse_grader_rejects_unknown_kind() {
        let err = parse_grader("id:bogus").unwrap_err();
        assert!(err.to_string().contains("bogus"), "{err}");
    }

    #[test]
    fn parse_grader_accepts_every_real_kind() {
        assert_eq!(
            parse_grader("a:deterministic").unwrap().kind,
            GraderKind::Deterministic
        );
        assert_eq!(parse_grader("b:agentic").unwrap().kind, GraderKind::Agentic);
        assert_eq!(parse_grader("c:human").unwrap().kind, GraderKind::Human);
    }

    #[test]
    fn slugify_lowercases_and_dashes() {
        assert_eq!(slugify("My New Eval v0!"), "my-new-eval-v0");
        assert_eq!(slugify("---"), "eval");
        assert_eq!(slugify(""), "eval");
    }

    #[test]
    fn resolve_out_path_defaults_to_evals_dir_with_id_slug() {
        let mut spec_json = serde_json::json!({"task": "code-review"});
        spec_json["id"] = serde_json::Value::String("My Eval V0".to_string());
        let spec: EvalSpec = serde_json::from_value(spec_json).unwrap();
        let path = resolve_out_path(None, &spec);
        assert_eq!(path, Path::new("evals/my-eval-v0.json"));
    }

    #[test]
    fn from_interactive_drives_a_full_prompt_benchmark_answer_set() {
        let script = "\n\
            code-review\n\
            \n\
            \n\
            \n\
            \n\
            prompt_benchmark\n\
            openrouter/auto\n\
            Answer exactly.\n\
            \n\
            \n\
            \n\
            marker-echo\n\
            Reply with crucible-smoke\n\
            \n\
            contains\n\
            crucible-smoke\n\
            \n";
        let mut reader = Cursor::new(script.as_bytes());
        let mut writer = Vec::new();
        let inputs = AuthorInputs::from_interactive(&mut reader, &mut writer).unwrap();
        let spec = inputs.into_eval_spec();
        assert_eq!(spec.task, "code-review");
        assert_eq!(spec.graders.graders.len(), 1);
        assert_eq!(spec.graders.graders[0].kind, GraderKind::Deterministic);
        let runner = spec.runner.expect("prompt_benchmark runner declared");
        assert_eq!(runner.kind, RunnerKind::PromptBenchmark);
    }

    #[test]
    fn from_interactive_reprompts_on_unknown_runner_kind_then_accepts() {
        let script = "\n\
            code-review\n\
            \n\
            \n\
            \n\
            \n\
            not-a-real-kind\n\
            key_recall\n\
            ../daedalus/arenas/pr-review-v0\n\
            ../daedalus/runs/freeze/trials.jsonl\n\
            probe-oneshot\n\
            \n\
            \n";
        let mut reader = Cursor::new(script.as_bytes());
        let mut writer = Vec::new();
        let inputs = AuthorInputs::from_interactive(&mut reader, &mut writer).unwrap();
        let spec = inputs.into_eval_spec();
        assert_eq!(
            spec.runner.expect("runner declared").kind,
            RunnerKind::KeyRecall
        );
        let out = String::from_utf8(writer).unwrap();
        assert!(out.contains("unknown runner kind"), "{out}");
    }

    #[test]
    fn validate_and_maybe_write_refuses_an_invalid_spec_and_leaves_no_file() {
        let dir = temp_dir("invalid");
        let mut args = base_args();
        args.task_family = Some("prompt-smoke".to_string());
        args.runner_kind = Some(AuthorRunnerKind::PromptBenchmark);
        args.prompt_model = Some("openrouter/auto".to_string());
        args.prompt_system_prompt = Some("Answer exactly.".to_string());
        args.prompt_task_id = Some("marker-echo".to_string());
        args.prompt_task_prompt = Some("Reply with crucible-smoke".to_string());
        args.prompt_expectation_kind = Some(AuthorExpectationKind::Contains);
        args.prompt_expectation_value = Some("crucible-smoke".to_string());
        // No deterministic grader named, and an explicit human grader that
        // doesn't satisfy prompt_benchmark's requirement.
        args.graders = vec!["operator:human".to_string()];

        let inputs = AuthorInputs::from_flags(&args).unwrap();
        let spec = inputs.into_eval_spec();
        let out_path = dir.join("bad-spec.json");

        let (report, written) = validate_and_maybe_write(&spec, &out_path).unwrap();
        assert!(!written, "an invalid spec must not be written");
        assert!(!report.valid);
        assert!(!out_path.exists(), "no file should exist at the out path");
        // No leftover scratch file either.
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert!(
            leftovers.is_empty(),
            "scratch file must be cleaned up: {leftovers:?}"
        );
    }

    #[test]
    fn validate_and_maybe_write_saves_a_valid_spec_that_reloads_identically() {
        let dir = temp_dir("valid");
        let mut args = base_args();
        args.task_family = Some("prompt-smoke".to_string());
        args.runner_kind = Some(AuthorRunnerKind::PromptBenchmark);
        args.prompt_model = Some("openrouter/auto".to_string());
        args.prompt_system_prompt = Some("Answer exactly.".to_string());
        args.prompt_task_id = Some("marker-echo".to_string());
        args.prompt_task_prompt = Some("Reply with crucible-smoke".to_string());
        args.prompt_expectation_kind = Some(AuthorExpectationKind::Contains);
        args.prompt_expectation_value = Some("crucible-smoke".to_string());

        let inputs = AuthorInputs::from_flags(&args).unwrap();
        let spec = inputs.into_eval_spec();
        let out_path = dir.join("prompt-smoke-v0.json");

        let (report, written) = validate_and_maybe_write(&spec, &out_path).unwrap();
        assert!(written, "{:?}", report.errors);
        assert!(report.valid, "{:?}", report.errors);
        assert!(report.runnable, "{:?}", report.errors);
        assert!(out_path.exists());

        let bytes = std::fs::read(&out_path).unwrap();
        let reloaded: EvalSpec = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reloaded, spec);
    }
}
