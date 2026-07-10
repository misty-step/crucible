//! `crucible run --env`: run one declared eval spec across several named
//! operating environments, then compare the results — the operator-driven eval
//! workbench's core loop.
//!
//! Each [`crucible_core::Environment`] is applied to the spec as a pure
//! transform (see the `environment` module), producing a spec that differs from
//! its siblings only on the invocation axes the environment overrides. Every
//! transformed spec runs through the unchanged execution + persistence stack,
//! so the resulting runs land in the ledger with distinct `config_id`s and are
//! compared through the same `runs compare` surface — paired stats, resolution,
//! and attribution intact. This module is deliberately thin: it loads, applies,
//! runs, persists, and renders. All the rigor lives in the layers it composes.
//!
//! All environment declarations are loaded, validated, and checked for
//! output-directory collisions **before any model call is made** — a malformed
//! or colliding environment fails the whole invocation up front, not after the
//! first environment has already spent money. The first environment is the
//! baseline; every later environment is compared against it. For the common
//! two-environment case (env A vs env B) that is exactly one comparison.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use crucible_core::{CorpusSpec, Environment, EvalSpec};
use serde::Serialize;

use crate::eval_run;
use crate::run_store;
use crate::spec_run::{self, RunOptions};

/// One environment prepared for execution: its identity, the spec transformed
/// by applying it, and the output directory it will run under — everything
/// resolved and validated before the first model call.
#[derive(Debug)]
struct PreparedEnv {
    env_id: String,
    description: String,
    model: Option<String>,
    transformed: EvalSpec,
    out_dir: PathBuf,
}

/// Run `spec` once per environment in `env_paths`, persist each run, and render
/// receipts plus a baseline-vs-challenger comparison for every environment past
/// the first.
pub(crate) fn run(
    spec: Option<&Path>,
    out: Option<&Path>,
    json: bool,
    strict_tracked: bool,
    env_paths: &[PathBuf],
    alpha: f64,
    db: &Path,
) -> anyhow::Result<()> {
    let spec_path = spec.with_context(|| "--env requires a declared eval spec")?;
    if env_paths.len() < 2 {
        anyhow::bail!(
            "--env expects at least two environments to compare (got {}); \
             to run a single environment, apply it and use `crucible run` directly",
            env_paths.len()
        );
    }

    let base_spec = spec_run::load_spec(spec_path)?;
    let base_out = match out {
        Some(out) => out.to_path_buf(),
        None => spec_run::default_output_dir(spec_path)?,
    };

    // Resolve and validate every environment before spending on any model
    // call: a malformed environment, an axis the runner can't accept, or an
    // output-directory collision fails the whole invocation here, not after
    // the first environment has already run and paid.
    let prepared = prepare(&base_spec, &base_out, env_paths)?;

    let mut receipts = Vec::with_capacity(prepared.len());
    let mut tracked_failures = Vec::new();
    for env in &prepared {
        let report = spec_run::run_loaded_spec(
            &env.transformed,
            spec_path,
            Some(&env.out_dir),
            &RunOptions::default(),
        )
        .with_context(|| format!("running eval in environment {:?}", env.env_id))?;
        let stored = run_store::persist_report(db, &report)?;
        if strict_tracked {
            tracked_failures.extend(spec_run::tracked_failures(&report)?);
        }

        let benchmark_id = report
            .evals
            .first()
            .map(|eval| eval.id.clone())
            .with_context(|| format!("environment {:?} produced no eval report", env.env_id))?;
        let run_id = format!("{}:{}", stored.invocation_id, benchmark_id);
        let config_id = run_store::show_run(db, &run_id)?.run.config_id;
        let score = report
            .evals
            .first()
            .map(|eval| eval_run::format_score(&eval.score))
            .unwrap_or_else(|| "n/a".to_string());

        receipts.push(EnvReceipt {
            env_id: env.env_id.clone(),
            description: env.description.clone(),
            model: env.model.clone(),
            benchmark_id,
            run_id,
            config_id,
            score,
            output_dir: report.output_dir,
        });
    }

    if json {
        print_json(db, &receipts, alpha)?;
    } else {
        print_human(db, &receipts, alpha)?;
    }
    if !tracked_failures.is_empty() {
        anyhow::bail!(
            "tracked checks failed: {}",
            spec_run::format_tracked_failures(&tracked_failures)
        );
    }
    Ok(())
}

/// Load, validate, transform, and collision-check every environment against
/// `base_spec` — the pre-spend validation pass. Returns the environments ready
/// to run, or the first failure.
fn prepare(
    base_spec: &EvalSpec,
    base_out: &Path,
    env_paths: &[PathBuf],
) -> anyhow::Result<Vec<PreparedEnv>> {
    // Collision key is the lowercased directory name: two env ids that differ
    // only by case (`GLM` vs `glm`) sanitize to distinct Rust strings but the
    // SAME physical directory on a case-insensitive filesystem (default macOS
    // APFS, Windows), where the second run would clobber the first's evidence.
    // Same guard the `--models` fanout uses (`run_fanout::model_output_dirs`).
    let mut seen_dirs: HashMap<String, String> = HashMap::new();
    let mut prepared = Vec::with_capacity(env_paths.len());
    for env_path in env_paths {
        let env = load_environment(env_path)?;
        let dir_name = env_dir_name(&env.id);
        if let Some(prior) = seen_dirs.insert(dir_name.to_ascii_lowercase(), env.id.clone()) {
            anyhow::bail!(
                "environments {:?} and {:?} collide in output directory {:?} (case-insensitive)",
                prior,
                env.id,
                dir_name
            );
        }
        let transformed = env
            .apply_to(base_spec)
            .with_context(|| format!("applying environment {:?}", env.id))?;
        let model = config_model(&env, &transformed);
        prepared.push(PreparedEnv {
            env_id: env.id,
            description: env.description,
            model,
            transformed,
            out_dir: base_out.join(&dir_name),
        });
    }
    Ok(prepared)
}

/// The model slug a run in this environment actually used: the environment's
/// override when present, otherwise whatever the transformed spec resolved to.
fn config_model(env: &Environment, transformed: &EvalSpec) -> Option<String> {
    if let Some(model) = &env.model {
        return Some(model.clone());
    }
    let runner = transformed.runner.as_ref()?;
    match &runner.corpus {
        CorpusSpec::PromptBenchmark { config, .. } => Some(config.model.clone()),
        CorpusSpec::AgenticJudge { config, .. } => Some(config.model.clone()),
        CorpusSpec::HarborTasks { config, .. } => config.model.clone(),
        _ => None,
    }
}

fn load_environment(path: &Path) -> anyhow::Result<Environment> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading environment declaration {}", path.display()))?;
    let env: Environment = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {} as a Crucible Environment", path.display()))?;
    env.validate()
        .with_context(|| format!("validating environment {}", path.display()))?;
    Ok(env)
}

fn env_dir_name(env_id: &str) -> String {
    let mut out = String::with_capacity(env_id.len() + 4);
    out.push_str("env-");
    for ch in env_id.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    out
}

#[derive(Debug, Clone, Serialize)]
struct EnvReceipt {
    env_id: String,
    description: String,
    model: Option<String>,
    benchmark_id: String,
    run_id: String,
    config_id: String,
    score: String,
    output_dir: String,
}

fn print_json(db: &Path, receipts: &[EnvReceipt], alpha: f64) -> anyhow::Result<()> {
    #[derive(Serialize)]
    struct MatrixReport<'a> {
        schema_version: &'static str,
        db: String,
        environments: &'a [EnvReceipt],
        comparisons: Vec<run_store::ConfigComparison>,
    }

    let comparisons = comparisons(db, receipts, alpha)?;
    let report = MatrixReport {
        schema_version: "crucible.run_matrix.v1",
        db: db.display().to_string(),
        environments: receipts,
        comparisons,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn print_human(db: &Path, receipts: &[EnvReceipt], alpha: f64) -> anyhow::Result<()> {
    println!("crucible run matrix");
    println!("  db       {}", db.display());
    for receipt in receipts {
        println!(
            "  env      {}  {}  model={}  out={}",
            receipt.env_id,
            receipt.score,
            receipt.model.as_deref().unwrap_or("(spec)"),
            receipt.output_dir,
        );
        println!("           config={}", receipt.config_id);
    }

    let Some((baseline, challengers)) = receipts.split_first() else {
        return Ok(());
    };
    for challenger in challengers {
        println!();
        println!(
            "comparison  baseline {} vs {}",
            baseline.env_id, challenger.env_id
        );
        let comparison = run_store::compare_configs(
            db,
            &baseline.benchmark_id,
            &baseline.config_id,
            &challenger.config_id,
            alpha,
            false,
        )?;
        crate::print_config_comparison(&comparison);
        println!(
            "  repro    {}",
            crate::runs_compare_repro_command(
                db,
                &baseline.benchmark_id,
                &baseline.config_id,
                &challenger.config_id,
                alpha,
            )
        );
    }
    Ok(())
}

fn comparisons(
    db: &Path,
    receipts: &[EnvReceipt],
    alpha: f64,
) -> anyhow::Result<Vec<run_store::ConfigComparison>> {
    let Some((baseline, challengers)) = receipts.split_first() else {
        return Ok(Vec::new());
    };
    challengers
        .iter()
        .map(|challenger| {
            run_store::compare_configs(
                db,
                &baseline.benchmark_id,
                &baseline.config_id,
                &challenger.config_id,
                alpha,
                false,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crucible_core::{
        Grader, GraderKind, GraderManifest, ModelProvider, PromptBenchmarkTask, PromptExpectation,
        PromptModelConfig, RunnerKind, RunnerSpec,
    };

    #[test]
    fn env_dir_name_sanitizes_and_prefixes() {
        assert_eq!(env_dir_name("glm-4.6"), "env-glm-4.6");
        assert_eq!(env_dir_name("gpt/4o mini"), "env-gpt-4o-mini");
        assert_eq!(env_dir_name("a_b.C-1"), "env-a_b.C-1");
    }

    fn spec() -> EvalSpec {
        EvalSpec {
            schema_version: crucible_core::EVAL_SPEC_SCHEMA.to_string(),
            id: "demo-v0".to_string(),
            title: None,
            context: None,
            task: "demo".to_string(),
            inputs: String::new(),
            outputs: String::new(),
            fixtures: Vec::new(),
            graders: GraderManifest {
                graders: vec![Grader {
                    id: "g".to_string(),
                    kind: GraderKind::Deterministic,
                }],
            },
            baselines: Vec::new(),
            aggregation: Default::default(),
            uncertainty: Default::default(),
            decision: String::new(),
            min_effect_of_interest: None,
            runner: Some(RunnerSpec {
                kind: RunnerKind::PromptBenchmark,
                corpus: CorpusSpec::PromptBenchmark {
                    config: PromptModelConfig {
                        provider: ModelProvider::OpenRouter,
                        model: "openrouter/auto".to_string(),
                        system_prompt: "sp".to_string(),
                        credential_env: "OPENROUTER_API_KEY".to_string(),
                        max_output_units: Some(8),
                        temperature: Some(0),
                        harness: None,
                        tool_allowlist: Vec::new(),
                    },
                    tasks: vec![PromptBenchmarkTask {
                        task_id: "t".to_string(),
                        class: None,
                        summary: None,
                        context_file: None,
                        prompt: "p".to_string(),
                        expectation: PromptExpectation::Contains {
                            value: "x".to_string(),
                        },
                        tracked: Vec::new(),
                    }],
                },
            }),
        }
    }

    fn write_env(dir: &Path, name: &str, id: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(
                r#"{{"schema_version":"crucible.environment.v1","id":"{id}","model":"x/{id}"}}"#
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn prepare_rejects_case_insensitive_output_directory_collisions() {
        let tmp = std::env::temp_dir().join(format!(
            "crucible-run-matrix-collide-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let a = write_env(&tmp, "a.json", "GLM-4.6");
        let b = write_env(&tmp, "b.json", "glm-4.6");
        let err = prepare(&spec(), &tmp, &[a, b])
            .expect_err("case-insensitive directory collisions must be rejected");
        assert!(
            err.to_string().contains("collide"),
            "error names the collision: {err}"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn prepare_transforms_each_environment_and_records_model() {
        let tmp = std::env::temp_dir().join(format!(
            "crucible-run-matrix-prepare-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let a = write_env(&tmp, "a.json", "deepseek");
        let b = write_env(&tmp, "b.json", "glm");
        let prepared = prepare(&spec(), &tmp, &[a, b]).unwrap();
        assert_eq!(prepared.len(), 2);
        assert_eq!(prepared[0].env_id, "deepseek");
        assert_eq!(prepared[0].model.as_deref(), Some("x/deepseek"));
        // The transform overrides the model but holds the spec's content.
        let CorpusSpec::PromptBenchmark { config, .. } =
            &prepared[1].transformed.runner.as_ref().unwrap().corpus
        else {
            panic!("expected prompt_benchmark corpus");
        };
        assert_eq!(config.model, "x/glm");
        assert_eq!(config.system_prompt, "sp");
        assert_eq!(prepared[1].out_dir, tmp.join("env-glm"));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
