//! `crucible import`: project an externally-authored eval/benchmark
//! definition into a Crucible `EvalSpec` (backlog/Powder crucible-026).
//!
//! Crucible exports Harbor-importable benchmark tasks (`VISION.md`) but,
//! until this command, had no path in the other direction: every external
//! benchmark someone else defined became a bespoke one-off script that never
//! touched the run ledger, calibration, or grader mix. `crucible import
//! <adapter> <source>` closes that gap for one real adapter — VISION.md's
//! explicit instruction not to reinvent eval infrastructure means this
//! projects a *borrowed* definition onto a runner Crucible already owns and
//! executes, rather than building a new engine.
//!
//! The first (and, for now, only) adapter is `promptfoo`: a
//! [Promptfoo](https://promptfoo.dev)-style YAML eval config projected onto
//! the `prompt_benchmark` runner. See [`crucible_core::import`] for the
//! actual projection logic and its total/honest contract — every test case
//! in the source config either becomes a task or is named in the printed
//! report as skipped, with why. This module is the CLI/I/O shell around
//! that: read the file, run the projection, assemble the `EvalSpec`, and run
//! it through the exact same validate-then-save gate `crucible author` uses
//! (`crate::spec_save`) — an assembly that fails validation, or that maps
//! zero importable tasks, is refused and leaves no file behind.

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use serde::Serialize;

use crucible_core::import::{parse_promptfoo_config, project_promptfoo, PromptfooConfig};
use crucible_core::{
    AggregationMethod, CorpusSpec, EvalSpec, Grader, GraderKind, GraderManifest, ModelProvider,
    PromptModelConfig, RunnerKind, RunnerSpec, SkippedTest, UncertaintyRule, EVAL_SPEC_SCHEMA,
};

use crate::spec_save::{resolve_out_path, slugify, validate_and_maybe_write};
use crate::validate::ValidationReport;

/// Schema identifier for `crucible import promptfoo --json`'s report.
pub const IMPORT_REPORT_SCHEMA: &str = "crucible.import_report.v1";

/// Flags for `crucible import promptfoo`.
#[derive(Debug, Args)]
pub struct PromptfooImportArgs {
    /// Path to the promptfoo config (`promptfooconfig.yaml`/`.json`).
    #[arg(value_name = "CONFIG")]
    pub config: PathBuf,

    /// Output path for the assembled spec JSON. Defaults to
    /// `evals/<id>.json`.
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,

    /// Overwrite an existing file at the output path. Without this, an
    /// existing file at `--out` refuses the write instead of silently
    /// clobbering a hand-authored or previously imported spec.
    #[arg(long)]
    pub force: bool,

    /// Emit the import + validation report as stable JSON instead of a
    /// readable summary.
    #[arg(long)]
    pub json: bool,

    /// Stable eval id. Defaults to `promptfoo-<slug of description or config
    /// filename>`.
    #[arg(long)]
    pub id: Option<String>,

    /// The task family this eval measures. Defaults to `promptfoo-import`.
    #[arg(long = "task-family", value_name = "TASK")]
    pub task_family: Option<String>,

    /// The decision this eval informs, in one sentence.
    #[arg(long)]
    pub decision: Option<String>,

    /// Shared system prompt for every imported task. Defaults to empty — the
    /// source config carries no separate system-prompt concept, so nothing
    /// is invented on its behalf unless this is set.
    #[arg(long = "system-prompt", value_name = "TEXT")]
    pub system_prompt: Option<String>,

    /// Override the OpenRouter model slug the config's declared provider(s)
    /// mapped to. Use this when the config names a provider Crucible cannot
    /// map (reported as `no usable provider`), or to point the imported spec
    /// at a different model than the source declared.
    #[arg(long, value_name = "SLUG")]
    pub model: Option<String>,

    /// Env var carrying the provider credential. Defaults to
    /// `OPENROUTER_API_KEY`.
    #[arg(long = "credential-env", value_name = "ENV")]
    pub credential_env: Option<String>,
}

/// `crucible import promptfoo`: read, project, assemble, validate, and (if
/// valid and non-empty) save.
pub fn run(args: PromptfooImportArgs) -> anyhow::Result<()> {
    let report = import_promptfoo(&args)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_import_report(&report);
    }
    if !report.written {
        anyhow::bail!(
            "import produced no runnable spec at {}; see the report above for why (refusing to write)",
            report.out
        );
    }
    Ok(())
}

/// One imported/skipped test in the stable JSON report shape.
#[derive(Debug, Serialize)]
pub struct SkippedTestReport {
    pub locator: String,
    pub reason: String,
}

impl From<SkippedTest> for SkippedTestReport {
    fn from(s: SkippedTest) -> Self {
        Self {
            locator: s.locator,
            reason: s.reason,
        }
    }
}

/// Stable JSON shape for `crucible import promptfoo --json`.
#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub schema_version: &'static str,
    pub source: String,
    pub out: String,
    pub written: bool,
    /// The model slug actually placed in the assembled spec — either the
    /// config's mapped provider, or `--model` when given.
    pub model: String,
    /// The model slug the config's declared provider(s) mapped to, before
    /// any `--model` override. Present even when `--model` was given, so the
    /// report never hides what the source actually declared.
    pub source_model: String,
    pub prompt_source: String,
    pub imported_count: usize,
    pub declared_test_count: usize,
    /// Total accounting: every test in the source config not present in
    /// `imported_count` is here, with why.
    pub skipped_tests: Vec<SkippedTestReport>,
    pub skipped_providers: Vec<String>,
    pub skipped_prompts: Vec<String>,
    pub validate: ValidationReport,
}

fn import_promptfoo(args: &PromptfooImportArgs) -> anyhow::Result<ImportReport> {
    let source_text = std::fs::read_to_string(&args.config)
        .with_context(|| format!("reading promptfoo config {}", args.config.display()))?;
    let config = parse_promptfoo_config(&source_text)
        .with_context(|| format!("parsing promptfoo config {}", args.config.display()))?;
    let base_dir = args
        .config
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let declared_test_count = config.tests.len();
    let projection = project_promptfoo(&config, base_dir)
        .with_context(|| format!("cannot import {}", args.config.display()))?;

    if projection.imported.is_empty() {
        // Total accounting: every skip reason is already on the report;
        // surface it in the refusal instead of a bare "nothing to run".
        let reasons: Vec<String> = projection
            .skipped_tests
            .iter()
            .map(|s| format!("{}: {}", s.locator, s.reason))
            .collect();
        anyhow::bail!(
            "no test case in {} could be imported ({declared_test_count} declared, all skipped): {}",
            args.config.display(),
            if reasons.is_empty() {
                "config declares no tests".to_string()
            } else {
                reasons.join("; ")
            }
        );
    }

    let model = args
        .model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| projection.model.clone());
    let credential_env = args
        .credential_env
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "OPENROUTER_API_KEY".to_string());
    let system_prompt = args.system_prompt.clone().unwrap_or_default();

    let id = args
        .id
        .clone()
        .unwrap_or_else(|| default_id(&config, &args.config));
    let task = args
        .task_family
        .clone()
        .unwrap_or_else(|| "promptfoo-import".to_string());
    let inputs = match &config.description {
        Some(d) if !d.trim().is_empty() => {
            format!(
                "Imported from promptfoo config {} ({d})",
                args.config.display()
            )
        }
        _ => format!("Imported from promptfoo config {}", args.config.display()),
    };

    let spec = EvalSpec {
        schema_version: EVAL_SPEC_SCHEMA.to_string(),
        id,
        title: None,
        context: None,
        task,
        inputs,
        outputs: "Deterministic rubric pass/fail per imported promptfoo test case".to_string(),
        fixtures: Vec::new(),
        graders: GraderManifest {
            graders: vec![Grader {
                id: "deterministic_rubric".to_string(),
                kind: GraderKind::Deterministic,
            }],
        },
        baselines: Vec::new(),
        aggregation: AggregationMethod::Proportion,
        uncertainty: UncertaintyRule::default(),
        decision: args.decision.clone().unwrap_or_default(),
        min_effect_of_interest: None,
        runner: Some(RunnerSpec {
            kind: RunnerKind::PromptBenchmark,
            corpus: CorpusSpec::PromptBenchmark {
                config: PromptModelConfig {
                    provider: ModelProvider::OpenRouter,
                    model,
                    system_prompt,
                    credential_env,
                    max_output_units: None,
                    temperature: None,
                    harness: None,
                    tool_allowlist: Vec::new(),
                },
                tasks: projection.imported,
            },
        }),
    };

    let out_path = resolve_out_path(args.out.as_deref(), &spec);
    if out_path.exists() && !args.force {
        anyhow::bail!(
            "refusing to overwrite existing spec at {} (pass --force to overwrite)",
            out_path.display()
        );
    }
    let imported_count = declared_test_count - projection.skipped_tests.len();
    let source_model = projection.model;
    let prompt_source = projection.prompt_source;
    let skipped_tests: Vec<SkippedTestReport> = projection
        .skipped_tests
        .into_iter()
        .map(SkippedTestReport::from)
        .collect();
    let skipped_providers = projection.skipped_providers;
    let skipped_prompts = projection.skipped_prompts;

    let model_written = match &spec
        .runner
        .as_ref()
        .expect("prompt_benchmark runner just assembled above")
        .corpus
    {
        CorpusSpec::PromptBenchmark { config, .. } => config.model.clone(),
        _ => unreachable!("runner assembled as PromptBenchmark above"),
    };

    let (validate_report, written) = validate_and_maybe_write(&spec, &out_path)?;

    Ok(ImportReport {
        schema_version: IMPORT_REPORT_SCHEMA,
        source: args.config.display().to_string(),
        out: out_path.display().to_string(),
        written,
        model: model_written,
        source_model,
        prompt_source,
        imported_count,
        declared_test_count,
        skipped_tests,
        skipped_providers,
        skipped_prompts,
        validate: validate_report,
    })
}

fn default_id(config: &PromptfooConfig, config_path: &Path) -> String {
    let slug = config
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(slugify)
        .unwrap_or_else(|| {
            config_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(slugify)
                .unwrap_or_else(|| "import".to_string())
        });
    format!("promptfoo-{slug}")
}

fn print_import_report(report: &ImportReport) {
    println!("crucible import promptfoo");
    println!("  source     {}", report.source);
    println!("  out        {}", report.out);
    if report.model == report.source_model {
        println!("  model      {}", report.model);
    } else {
        println!(
            "  model      {} (source config mapped to {})",
            report.model, report.source_model
        );
    }
    println!("  prompt     {}", report.prompt_source);
    println!(
        "  imported   {}/{} test case(s)",
        report.imported_count, report.declared_test_count
    );
    for s in &report.skipped_tests {
        println!("  skipped    {} — {}", s.locator, s.reason);
    }
    for p in &report.skipped_providers {
        println!("  skipped    provider {p}");
    }
    for p in &report.skipped_prompts {
        println!("  skipped    prompt {p}");
    }
    println!("  valid      {}", report.validate.valid);
    println!("  runnable   {}", report.validate.runnable);
    for error in &report.validate.errors {
        println!("  ERROR      {}: {}", error.field, error.message);
    }
    for warning in &report.validate.warnings {
        println!("  warning    {}: {}", warning.field, warning.message);
    }
    if report.written {
        println!("  wrote      {}", report.out);
    } else {
        println!("  refused    spec failed validation; nothing written");
    }
}
