use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use serde::Serialize;

use crate::eval_run::{self, RunEval, RunReport};
use crate::run_store::{self, PersistedReport};
use crate::spec_run;

pub(crate) fn run(
    spec: Option<&Path>,
    eval: RunEval,
    out: Option<&Path>,
    json: bool,
    models: &str,
    db: &Path,
) -> anyhow::Result<()> {
    let spec_path = spec.with_context(|| "--models requires a declared prompt_benchmark spec")?;
    if eval != RunEval::All {
        anyhow::bail!("--eval selects built-in receipts and cannot be combined with --models");
    }

    let models = parse_model_list(models)?;
    let output_dirs = model_output_dirs(&models)?;
    let base_out = match out {
        Some(out) => out.to_path_buf(),
        None => spec_run::default_output_dir(spec_path)?,
    };

    let mut receipts = Vec::new();
    for (model, output_dir) in models.into_iter().zip(output_dirs) {
        receipts.push(run_one_model(spec_path, &base_out, db, model, &output_dir)?);
    }

    if json {
        print_json(db, &receipts)?;
    } else {
        print_human(db, &receipts);
    }
    Ok(())
}

fn run_one_model(
    spec_path: &Path,
    base_out: &Path,
    db: &Path,
    model: String,
    output_dir: &str,
) -> anyhow::Result<ModelReceipt> {
    let model_out = base_out.join(output_dir);
    let options = spec_run::RunOptions::with_prompt_model(&model);
    let report = spec_run::run_with_options(spec_path, Some(&model_out), &options)?;
    let stored = run_store::persist_report(db, &report)?;
    Ok(ModelReceipt {
        model,
        report,
        stored,
    })
}

#[derive(Debug)]
struct ModelReceipt {
    model: String,
    report: RunReport,
    stored: PersistedReport,
}

fn print_json(db: &Path, receipts: &[ModelReceipt]) -> anyhow::Result<()> {
    #[derive(Serialize)]
    struct FanoutReport<'a> {
        schema_version: &'static str,
        db: String,
        runs: Vec<FanoutRun<'a>>,
    }

    #[derive(Serialize)]
    struct FanoutRun<'a> {
        model: &'a str,
        output_dir: &'a str,
        run_report: String,
        invocation_id: &'a str,
        run_records: usize,
        prompt_task_results: usize,
    }

    let report = FanoutReport {
        schema_version: "crucible.run_fanout.v1",
        db: db.display().to_string(),
        runs: receipts
            .iter()
            .map(|receipt| FanoutRun {
                model: &receipt.model,
                output_dir: &receipt.report.output_dir,
                run_report: Path::new(&receipt.report.output_dir)
                    .join("run-report.json")
                    .display()
                    .to_string(),
                invocation_id: &receipt.stored.invocation_id,
                run_records: receipt.stored.run_records,
                prompt_task_results: receipt.stored.prompt_task_results,
            })
            .collect(),
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn print_human(db: &Path, receipts: &[ModelReceipt]) {
    println!("crucible run fanout");
    println!("  db       {}", db.display());
    for receipt in receipts {
        let score = receipt
            .report
            .evals
            .first()
            .map(|eval| eval_run::format_score(&eval.score))
            .unwrap_or_else(|| "n/a".to_string());
        println!(
            "  model    {}  {}  out={}  stored={} row{}, {} prompt task row{}",
            receipt.model,
            score,
            receipt.report.output_dir,
            receipt.stored.run_records,
            plural(receipt.stored.run_records),
            receipt.stored.prompt_task_results,
            plural(receipt.stored.prompt_task_results)
        );
    }
}

fn parse_model_list(models: &str) -> anyhow::Result<Vec<String>> {
    let parsed: Vec<String> = models
        .split(',')
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .collect();
    if parsed.is_empty() {
        anyhow::bail!("--models requires at least one non-empty model slug");
    }
    Ok(parsed)
}

fn model_dir_name(model: &str) -> String {
    let mut out = String::with_capacity(model.len());
    for ch in model.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "model".to_string()
    } else {
        trimmed.to_string()
    }
}

fn model_output_dirs(models: &[String]) -> anyhow::Result<Vec<String>> {
    let mut by_dir: HashMap<String, String> = HashMap::new();
    let mut dirs = Vec::with_capacity(models.len());
    for model in models {
        let dir = model_dir_name(model);
        let collision_key = dir.to_ascii_lowercase();
        if let Some(existing) = by_dir.insert(collision_key, model.clone()) {
            anyhow::bail!(
                "--models contains slugs that collide in output directory {:?}: {:?} and {:?}",
                dir,
                existing,
                model
            );
        }
        dirs.push(dir);
    }
    Ok(dirs)
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_separated_models() {
        assert_eq!(
            parse_model_list(" deepseek/a, z-ai/b ,,moonshot/c ").unwrap(),
            vec!["deepseek/a", "z-ai/b", "moonshot/c"]
        );
    }

    #[test]
    fn model_slug_becomes_a_safe_directory_name() {
        assert_eq!(model_dir_name("z-ai/glm-5.2"), "z-ai-glm-5.2");
        assert_eq!(model_dir_name("///"), "model");
    }

    #[test]
    fn rejects_colliding_model_output_directories() {
        let err = model_output_dirs(&["a/b".to_string(), "a-b".to_string()])
            .expect_err("sanitized directory collisions must be rejected");
        assert!(
            err.to_string().contains("collide"),
            "error names the collision: {err}"
        );
    }

    #[test]
    fn rejects_case_insensitive_model_output_directory_collisions() {
        let err = model_output_dirs(&[
            "DeepSeek/deepseek-v4-flash".to_string(),
            "deepseek/deepseek-v4-flash".to_string(),
        ])
        .expect_err("case-insensitive directory collisions must be rejected");
        assert!(
            err.to_string().contains("collide"),
            "error names the collision: {err}"
        );
    }
}
