//! `crucible import harbor`: project a local directory of Harbor task
//! directories into a Crucible `harbor_task` `EvalSpec` (backlog/Powder
//! crucible-034).
//!
//! Scope is a representative, local, CPU-only smoke subset — a directory the
//! caller already populated with Harbor task directories (each carrying a
//! `task.toml`) — not the full ~100-task official Terminal-Bench 2.0 dataset,
//! which the crucible-034 design receipt scopes as an explicit follow-up card
//! (Docker/GPU footprint and `--n-concurrent` fan-out are a different build).
//! Mirrors `crucible import promptfoo`'s total/honest contract: every
//! directory entry is either imported or reported as skipped, with why, never
//! silently dropped.

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use serde::Serialize;

use crucible_core::{
    AggregationMethod, CorpusSpec, EvalSpec, Grader, GraderKind, GraderManifest, HarborRunConfig,
    HarborTaskSpec, RunnerKind, RunnerSpec, UncertaintyRule, EVAL_SPEC_SCHEMA,
};

use crate::spec_save::{resolve_out_path, slugify, validate_and_maybe_write};
use crate::validate::ValidationReport;

/// Schema identifier for `crucible import harbor --json`'s report.
pub const HARBOR_IMPORT_REPORT_SCHEMA: &str = "crucible.harbor_import_report.v1";

/// Flags for `crucible import harbor`.
#[derive(Debug, Args)]
pub struct HarborImportArgs {
    /// Directory containing Harbor task subdirectories, each with its own
    /// `task.toml`.
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,

    /// Output path for the assembled spec JSON. Defaults to
    /// `evals/<id>.json`.
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,

    /// Overwrite an existing file at the output path.
    #[arg(long)]
    pub force: bool,

    /// Emit the import + validation report as stable JSON instead of a
    /// readable summary.
    #[arg(long)]
    pub json: bool,

    /// Stable eval id. Defaults to `harbor-<slug of the directory name>`.
    #[arg(long)]
    pub id: Option<String>,

    /// The task family this eval measures. Defaults to `harbor-import`.
    #[arg(long = "task-family", value_name = "TASK")]
    pub task_family: Option<String>,

    /// The decision this eval informs, in one sentence.
    #[arg(long)]
    pub decision: Option<String>,

    /// Harbor agent every imported task runs with. Defaults to `oracle`
    /// (applies the task's reference solution at zero model cost) — the
    /// right default for proving the seam works, not a real coding-agent
    /// eval; pass a real agent explicitly once that's the goal.
    #[arg(long, default_value = "oracle")]
    pub agent: String,

    /// Model slug for agents that need one. Omitted for agents like `oracle`
    /// that don't call a model.
    #[arg(long)]
    pub model: Option<String>,
}

/// `crucible import harbor`: scan, project, assemble, validate, and (if valid
/// and non-empty) save.
pub fn run(args: HarborImportArgs) -> anyhow::Result<()> {
    let report = import_harbor(&args)?;
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

/// One directory entry that could **not** be imported: which entry, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SkippedEntry {
    locator: String,
    reason: String,
}

/// Stable JSON shape for one [`SkippedEntry`].
#[derive(Debug, Serialize)]
pub struct SkippedEntryReport {
    pub locator: String,
    pub reason: String,
}

/// Stable JSON shape for `crucible import harbor --json`.
#[derive(Debug, Serialize)]
pub struct HarborImportReport {
    pub schema_version: &'static str,
    pub source: String,
    pub out: String,
    pub written: bool,
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub imported_count: usize,
    pub declared_entry_count: usize,
    /// Total accounting: every directory entry not present in
    /// `imported_count` is here, with why.
    pub skipped: Vec<SkippedEntryReport>,
    pub validate: ValidationReport,
}

fn import_harbor(args: &HarborImportArgs) -> anyhow::Result<HarborImportReport> {
    let (tasks, skipped, declared_entry_count) = project_harbor_dir(&args.dir)
        .with_context(|| format!("scanning harbor task directory {}", args.dir.display()))?;

    if tasks.is_empty() {
        let reasons: Vec<String> = skipped
            .iter()
            .map(|s| format!("{}: {}", s.locator, s.reason))
            .collect();
        anyhow::bail!(
            "no Harbor task directory under {} could be imported ({declared_entry_count} entries, all skipped): {}",
            args.dir.display(),
            if reasons.is_empty() {
                "directory is empty".to_string()
            } else {
                reasons.join("; ")
            }
        );
    }

    let id = args.id.clone().unwrap_or_else(|| default_id(&args.dir));
    let task = args
        .task_family
        .clone()
        .unwrap_or_else(|| "harbor-import".to_string());

    let mut spec = EvalSpec {
        schema_version: EVAL_SPEC_SCHEMA.to_string(),
        id,
        title: None,
        context: None,
        task,
        inputs: format!(
            "Imported Harbor task directories from {}",
            args.dir.display()
        ),
        outputs: "Harbor's own verifier reward per task (reward >= 1.0 counted as pass)"
            .to_string(),
        fixtures: Vec::new(),
        graders: GraderManifest {
            graders: vec![Grader {
                id: "harbor_verifier".to_string(),
                kind: GraderKind::Deterministic,
            }],
        },
        baselines: Vec::new(),
        aggregation: AggregationMethod::Proportion,
        uncertainty: UncertaintyRule::default(),
        decision: args.decision.clone().unwrap_or_default(),
        min_effect_of_interest: None,
        runner: Some(RunnerSpec {
            kind: RunnerKind::HarborTask,
            corpus: CorpusSpec::HarborTasks {
                config: HarborRunConfig {
                    agent: args.agent.clone(),
                    model: args.model.clone(),
                    job_timeout_ms: None,
                    resource_envelope: None,
                },
                tasks,
            },
        }),
    };

    let out_path = resolve_out_path(args.out.as_deref(), &spec);
    rebase_task_dirs(&mut spec, &out_path)?;
    if out_path.exists() && !args.force {
        anyhow::bail!(
            "refusing to overwrite existing spec at {} (pass --force to overwrite)",
            out_path.display()
        );
    }

    let imported_count = declared_entry_count - skipped.len();
    let skipped_report: Vec<SkippedEntryReport> = skipped
        .into_iter()
        .map(|s| SkippedEntryReport {
            locator: s.locator,
            reason: s.reason,
        })
        .collect();

    let (validate_report, written) = validate_and_maybe_write(&spec, &out_path)?;

    Ok(HarborImportReport {
        schema_version: HARBOR_IMPORT_REPORT_SCHEMA,
        source: args.dir.display().to_string(),
        out: out_path.display().to_string(),
        written,
        agent: args.agent.clone(),
        model: args.model.clone(),
        imported_count,
        declared_entry_count,
        skipped: skipped_report,
        validate: validate_report,
    })
}

fn default_id(dir: &Path) -> String {
    let slug = dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(slugify)
        .unwrap_or_else(|| "import".to_string());
    format!("harbor-{slug}")
}

fn rebase_task_dirs(spec: &mut EvalSpec, out_path: &Path) -> anyhow::Result<()> {
    let out_parent = out_path
        .parent()
        .context("Harbor import output path has no parent directory")?;
    let out_parent = std::path::absolute(out_parent)
        .with_context(|| format!("resolving output directory {}", out_parent.display()))?;
    let Some(RunnerSpec {
        corpus: CorpusSpec::HarborTasks { tasks, .. },
        ..
    }) = spec.runner.as_mut()
    else {
        return Ok(());
    };

    for task in tasks {
        let absolute = std::path::absolute(&task.task_dir)
            .with_context(|| format!("resolving Harbor task path {}", task.task_dir))?;
        task.task_dir = relative_path(&out_parent, &absolute)
            .unwrap_or(absolute)
            .display()
            .to_string();
    }
    Ok(())
}

fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    let base_components: Vec<_> = base.components().collect();
    let target_components: Vec<_> = target.components().collect();
    let common = base_components
        .iter()
        .zip(&target_components)
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0 {
        return None;
    }

    let mut relative = PathBuf::new();
    for _ in &base_components[common..] {
        relative.push("..");
    }
    for component in &target_components[common..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Some(relative)
}

/// Scan `dir` for Harbor task subdirectories and project each into a
/// [`HarborTaskSpec`]. Total and honest: every entry in `dir` is either
/// imported or reported in the returned skip list, with why — never silently
/// dropped. A task directory is recognized by a readable `task.toml` that
/// declares a `[task]` section; this is a lightweight sanity check, not a
/// full TOML schema validation (Harbor itself validates the file's full
/// shape at `harbor run` time) — deliberately no new TOML-parsing dependency
/// for a check this narrow. Harbor 0.13's generated task format has a top-level
/// `version` plus `[verifier]`, `[agent]`, and `[environment]` sections; older
/// local tasks used a `[task]` section. Recognize both without pretending this
/// lightweight importer is Harbor's schema validator.
fn project_harbor_dir(
    dir: &Path,
) -> anyhow::Result<(Vec<HarborTaskSpec>, Vec<SkippedEntry>, usize)> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading directory {}", dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("reading directory entries under {}", dir.display()))?
        .into_iter()
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    let declared_entry_count = entries.len();

    let mut tasks = Vec::new();
    let mut skipped = Vec::new();
    for path in entries {
        let locator = path.display().to_string();
        if !path.is_dir() {
            skipped.push(SkippedEntry {
                locator,
                reason: "not a directory".to_string(),
            });
            continue;
        }
        let Some(task_id) = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            skipped.push(SkippedEntry {
                locator,
                reason: "directory name is not valid UTF-8".to_string(),
            });
            continue;
        };
        let toml_text = match std::fs::read_to_string(path.join("task.toml")) {
            Ok(text) => text,
            Err(_) => {
                skipped.push(SkippedEntry {
                    locator,
                    reason: "no readable task.toml found".to_string(),
                });
                continue;
            }
        };
        if !looks_like_harbor_task_toml(&toml_text) {
            skipped.push(SkippedEntry {
                locator,
                reason: "task.toml matches neither the legacy [task] shape nor the current version + [verifier]/[agent]/[environment] shape".to_string(),
            });
            continue;
        }
        tasks.push(HarborTaskSpec {
            task_id,
            task_dir: path.display().to_string(),
        });
    }
    Ok((tasks, skipped, declared_entry_count))
}

fn looks_like_harbor_task_toml(text: &str) -> bool {
    let mut has_version = false;
    let mut has_verifier = false;
    let mut has_agent = false;
    let mut has_environment = false;

    for raw_line in text.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line == "[task]" {
            return true;
        }
        if line
            .split_once('=')
            .is_some_and(|(key, value)| key.trim() == "version" && !value.trim().is_empty())
        {
            has_version = true;
        }
        match line {
            "[verifier]" => has_verifier = true,
            "[agent]" => has_agent = true,
            "[environment]" => has_environment = true,
            _ => {}
        }
    }

    has_version && has_verifier && has_agent && has_environment
}

fn print_import_report(report: &HarborImportReport) {
    println!("crucible import harbor");
    println!("  source     {}", report.source);
    println!("  out        {}", report.out);
    println!("  agent      {}", report.agent);
    if let Some(model) = &report.model {
        println!("  model      {model}");
    }
    println!(
        "  imported   {}/{} task directory(ies)",
        report.imported_count, report.declared_entry_count
    );
    for s in &report.skipped {
        println!("  skipped    {} — {}", s.locator, s.reason);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crucible-harbor-import-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_task_dir(root: &Path, name: &str, task_toml: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("task.toml"), task_toml).unwrap();
    }

    #[test]
    fn imports_every_valid_task_directory() {
        let root = temp_dir("import-all");
        write_task_dir(&root, "crucible-smoke", "[task]\nname=\"x\"\n");
        write_task_dir(&root, "another-task", "[task]\nname=\"y\"\n");

        let (tasks, skipped, declared) = project_harbor_dir(&root).unwrap();
        assert_eq!(declared, 2);
        assert_eq!(skipped.len(), 0);
        let mut task_ids: Vec<_> = tasks.iter().map(|t| t.task_id.clone()).collect();
        task_ids.sort();
        assert_eq!(task_ids, vec!["another-task", "crucible-smoke"]);
        assert!(tasks
            .iter()
            .all(|t| Path::new(&t.task_dir).join("task.toml").exists()));
    }

    #[test]
    fn imports_the_current_harbor_generated_task_shape() {
        let root = temp_dir("import-current-shape");
        write_task_dir(
            &root,
            "current-task",
            r#"version = "1.0"

[metadata]

[verifier]
timeout_sec = 900.0

[agent]
timeout_sec = 900.0

[environment]
build_timeout_sec = 600.0
"#,
        );

        let (tasks, skipped, declared) = project_harbor_dir(&root).unwrap();
        assert_eq!(declared, 1);
        assert!(skipped.is_empty());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_id, "current-task");
    }

    #[test]
    fn skips_a_non_directory_entry() {
        let root = temp_dir("import-file");
        std::fs::write(root.join("README.md"), "not a task").unwrap();
        let (tasks, skipped, declared) = project_harbor_dir(&root).unwrap();
        assert_eq!(declared, 1);
        assert_eq!(tasks.len(), 0);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("not a directory"));
    }

    #[test]
    fn skips_a_directory_with_no_task_toml() {
        let root = temp_dir("import-no-toml");
        std::fs::create_dir_all(root.join("empty-dir")).unwrap();
        let (tasks, skipped, _) = project_harbor_dir(&root).unwrap();
        assert_eq!(tasks.len(), 0);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("task.toml"));
    }

    #[test]
    fn skips_a_directory_with_a_malformed_task_toml() {
        let root = temp_dir("import-bad-toml");
        write_task_dir(&root, "bad-task", "not a task toml at all\n");
        let (tasks, skipped, _) = project_harbor_dir(&root).unwrap();
        assert_eq!(tasks.len(), 0);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("current"));
    }

    #[test]
    fn current_shape_requires_all_execution_sections() {
        let root = temp_dir("import-incomplete-current-shape");
        write_task_dir(
            &root,
            "incomplete-task",
            "version = \"1.0\"\n[verifier]\n[agent]\n",
        );

        let (tasks, skipped, _) = project_harbor_dir(&root).unwrap();
        assert!(tasks.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("[environment]"));
    }

    #[test]
    fn total_accounting_every_entry_is_imported_or_skipped() {
        let root = temp_dir("import-total");
        write_task_dir(&root, "good-task", "[task]\nname=\"x\"\n");
        std::fs::write(root.join("stray.txt"), "not a task").unwrap();
        let (tasks, skipped, declared) = project_harbor_dir(&root).unwrap();
        assert_eq!(declared, 2);
        assert_eq!(tasks.len() + skipped.len(), declared);
    }

    #[test]
    fn import_harbor_assembles_and_writes_a_valid_spec() {
        let root = temp_dir("import-e2e");
        let task_dir = root.join("tasks");
        write_task_dir(
            &task_dir,
            "crucible-smoke",
            "[task]\nname=\"misty-step/crucible-smoke\"\n",
        );
        let out_path = root.join("evals").join("harbor-smoke.json");

        let args = HarborImportArgs {
            dir: task_dir,
            out: Some(out_path.clone()),
            force: false,
            json: false,
            id: Some("harbor-smoke-test".to_string()),
            task_family: None,
            decision: None,
            agent: "oracle".to_string(),
            model: None,
        };
        let report = import_harbor(&args).expect("import harbor");
        assert!(
            report.written,
            "valid single-task import should write: {report:?}"
        );
        assert_eq!(report.imported_count, 1);
        assert_eq!(report.declared_entry_count, 1);
        assert!(out_path.exists());

        let written: EvalSpec =
            serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
        let runner = written.runner.expect("runner declared");
        assert_eq!(runner.kind, RunnerKind::HarborTask);
        let CorpusSpec::HarborTasks { config, tasks } = runner.corpus else {
            panic!("expected harbor_tasks corpus");
        };
        assert_eq!(config.agent, "oracle");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_id, "crucible-smoke");
        let resolved_task = out_path.parent().unwrap().join(&tasks[0].task_dir);
        assert!(
            resolved_task.join("task.toml").is_file(),
            "imported task path must resolve from the generated spec: {}",
            resolved_task.display()
        );
    }

    #[test]
    fn relative_path_rebases_a_sibling_tree_without_machine_specific_prefixes() {
        let base = Path::new("/workspace/bench/evals");
        let target = Path::new("/workspace/bench/tasks/one");
        assert_eq!(
            relative_path(base, target).unwrap(),
            PathBuf::from("../tasks/one")
        );
    }
}
