//! `crucible run --prompt-variant`: run one prompt-benchmark corpus under
//! named system-prompt variants and compare the persisted runs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use crucible_core::{CorpusSpec, EvalSpec};
use serde::Serialize;

use crate::eval_run;
use crate::run_store;
use crate::spec_run::{self, RunOptions};

/// Run a declared prompt benchmark once per selected named variant. The first
/// variant is the baseline; every later variant is a challenger. All validation,
/// transform, and output collision checks happen before the first model call.
pub(crate) fn run(
    spec: Option<&Path>,
    out: Option<&Path>,
    json: bool,
    strict_tracked: bool,
    requested: &[String],
    alpha: f64,
    db: &Path,
) -> anyhow::Result<()> {
    let spec_path = spec.with_context(|| "--prompt-variant requires a declared eval spec")?;
    let base_spec = spec_run::load_spec(spec_path)?;
    let variants = select_variants(&base_spec, requested)?;
    let base_out = match out {
        Some(out) => out.to_path_buf(),
        None => spec_run::default_output_dir(spec_path)?,
    };
    let prepared = prepare(&base_spec, &base_out, &variants)?;

    let mut receipts = Vec::with_capacity(prepared.len());
    let mut tracked_failures = Vec::new();
    for variant in &prepared {
        let options = RunOptions::with_prompt_variant(&variant.id);
        let report = spec_run::run_loaded_spec(
            &variant.transformed,
            spec_path,
            Some(&variant.out_dir),
            &options,
        )
        .with_context(|| format!("running prompt variant {:?}", variant.id))?;
        let stored = run_store::persist_report(db, &report)?;
        if strict_tracked {
            tracked_failures.extend(spec_run::tracked_failures(&report)?);
        }
        let benchmark_id = report
            .evals
            .first()
            .map(|eval| eval.id.clone())
            .with_context(|| format!("prompt variant {:?} produced no eval report", variant.id))?;
        let run_id = format!("{}:{}", stored.invocation_id, benchmark_id);
        let detail = run_store::show_run(db, &run_id)?;
        let score = report
            .evals
            .first()
            .map(|eval| eval_run::format_score(&eval.score))
            .unwrap_or_else(|| "n/a".to_string());
        receipts.push(VariantReceipt {
            id: variant.id.clone(),
            benchmark_id,
            run_id,
            config_id: detail.run.config_id,
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

#[derive(Debug)]
struct PreparedVariant {
    id: String,
    transformed: EvalSpec,
    out_dir: PathBuf,
}

fn prepare(
    base_spec: &EvalSpec,
    base_out: &Path,
    ids: &[String],
) -> anyhow::Result<Vec<PreparedVariant>> {
    let mut seen_dirs: HashMap<String, String> = HashMap::new();
    ids.iter()
        .map(|id| {
            let dir_name = variant_dir_name(id);
            if let Some(prior) = seen_dirs.insert(dir_name.to_ascii_lowercase(), id.clone()) {
                anyhow::bail!(
                    "prompt variants {:?} and {:?} collide in output directory {:?} (case-insensitive)",
                    prior,
                    id,
                    dir_name
                );
            }
            let transformed = base_spec
                .apply_prompt_variant(id)
                .with_context(|| format!("applying prompt variant {:?}", id))?;
            Ok(PreparedVariant {
                id: id.clone(),
                transformed,
                out_dir: base_out.join(dir_name),
            })
        })
        .collect()
}

fn select_variants(spec: &EvalSpec, requested: &[String]) -> anyhow::Result<Vec<String>> {
    let runner = spec
        .runner
        .as_ref()
        .with_context(|| "--prompt-variant requires a spec with a runner")?;
    let CorpusSpec::PromptBenchmark { config, .. } = &runner.corpus else {
        anyhow::bail!(
            "--prompt-variant is supported only for a prompt_benchmark runner, not {:?}",
            runner.kind
        );
    };
    if runner.kind != crucible_core::RunnerKind::PromptBenchmark {
        anyhow::bail!(
            "--prompt-variant is supported only for a prompt_benchmark runner, not {:?}",
            runner.kind
        );
    }
    spec.validate_prompt_variants()
        .context("validating prompt variant declaration")?;
    if config.prompt_variants.len() < 2 {
        anyhow::bail!(
            "--prompt-variant requires at least two named variants in the spec; found {}",
            config.prompt_variants.len()
        );
    }
    let declared: Vec<String> = config
        .prompt_variants
        .iter()
        .map(|variant| variant.id.clone())
        .collect();
    if requested.is_empty() {
        return Ok(declared);
    }
    let mut tokens = Vec::new();
    for raw in requested {
        tokens.extend(
            raw.split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
        );
    }
    if tokens.is_empty() {
        anyhow::bail!("--prompt-variant requires at least one non-empty variant id");
    }
    if tokens.len() == 1 && tokens[0] == "all" {
        return Ok(declared);
    }
    if tokens.iter().any(|id| id == "all") {
        anyhow::bail!("--prompt-variant all cannot be combined with named variants");
    }
    let mut selected = Vec::new();
    for id in tokens {
        if !declared.iter().any(|declared_id| declared_id == &id) {
            anyhow::bail!("prompt variant {:?} is not declared in the spec", id);
        }
        if !selected.contains(&id) {
            selected.push(id);
        }
    }
    if selected.len() < 2 {
        anyhow::bail!(
            "--prompt-variant needs at least two distinct variants for a paired comparison"
        );
    }
    Ok(selected)
}

fn variant_dir_name(id: &str) -> String {
    let mut out = String::with_capacity(id.len() + 7);
    out.push_str("prompt-");
    for ch in id.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    out
}

#[derive(Debug, Clone, Serialize)]
struct VariantReceipt {
    id: String,
    benchmark_id: String,
    run_id: String,
    config_id: String,
    score: String,
    output_dir: String,
}

fn comparisons(
    db: &Path,
    receipts: &[VariantReceipt],
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

fn print_json(db: &Path, receipts: &[VariantReceipt], alpha: f64) -> anyhow::Result<()> {
    #[derive(Serialize)]
    struct MatrixReport<'a> {
        schema_version: &'static str,
        db: String,
        variants: &'a [VariantReceipt],
        comparisons: Vec<run_store::ConfigComparison>,
    }
    let report = MatrixReport {
        schema_version: "crucible.prompt_variant_matrix.v1",
        db: db.display().to_string(),
        variants: receipts,
        comparisons: comparisons(db, receipts, alpha)?,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn print_human(db: &Path, receipts: &[VariantReceipt], alpha: f64) -> anyhow::Result<()> {
    println!("crucible run prompt variants");
    println!("  db       {}", db.display());
    for receipt in receipts {
        println!(
            "  variant  {}  {}  out={}",
            receipt.id, receipt.score, receipt.output_dir
        );
        println!("           config={}", receipt.config_id);
    }
    let Some((baseline, challengers)) = receipts.split_first() else {
        return Ok(());
    };
    for challenger in challengers {
        println!();
        println!("comparison  baseline {} vs {}", baseline.id, challenger.id);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_spec() -> EvalSpec {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .join("evals/prompt-variant-v0.json");
        spec_run::load_spec(&path).expect("load prompt variant fixture")
    }

    #[test]
    fn variant_directory_names_are_safe_and_prefixed() {
        assert_eq!(variant_dir_name("skill-on"), "prompt-skill-on");
        assert_eq!(variant_dir_name("bare/prompt"), "prompt-bare-prompt");
    }

    #[test]
    fn selector_supports_omitted_all_explicit_all_and_selected_variants() {
        let spec = fixture_spec();
        let declared = vec!["skill_off".to_string(), "skill_on".to_string()];
        assert_eq!(select_variants(&spec, &[]).unwrap(), declared);
        assert_eq!(
            select_variants(&spec, &["all".to_string()]).unwrap(),
            declared
        );
        assert_eq!(
            select_variants(&spec, &["skill_on,skill_off".to_string()]).unwrap(),
            vec!["skill_on", "skill_off"]
        );
    }

    #[test]
    fn selector_rejects_explicit_empty_and_mixed_all_tokens() {
        let spec = fixture_spec();
        for requested in [vec![String::new()], vec![",,".to_string()]] {
            let error = select_variants(&spec, &requested).expect_err("empty selector must fail");
            assert!(error
                .to_string()
                .contains("at least one non-empty variant id"));
        }
        let error = select_variants(&spec, &["all,skill_off".to_string()])
            .expect_err("all cannot mix with named variants");
        assert!(error.to_string().contains("cannot be combined"));
    }
}
