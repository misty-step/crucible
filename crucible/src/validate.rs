//! `crucible validate <spec>`: is a declared `EvalSpec` an executable
//! contract or aspirational metadata (backlog 014)?
//!
//! Every check here mirrors a real refusal `crucible run` enforces — see
//! [`crate::spec_run::preflight_spec`], which this module calls directly so
//! the two can never drift. Validation adds exactly one thing `run` cannot:
//! it works without a runnable corpus (no sibling checkout, no trials file,
//! no `OPENROUTER_API_KEY`), so a cold agent can check a spec is well-formed
//! before it has assembled real inputs. Non-fatal `warnings` cover fields
//! that are honestly *not yet* enforced (`baselines`) or are informational
//! (non-portable sibling-repo corpus paths) — reported, not hidden, rather
//! than either silently ignored or turned into a breaking refusal for a spec
//! that works today.

use std::path::Path;

use serde::Serialize;

use crate::spec_run::{
    check_prompt_regexes, load_spec, preflight_spec, resolve_spec_path_with_alias,
};
use crucible_core::{CorpusSpec, EvalSpec, RunnerKind};

/// Schema identifier for a persisted [`ValidationReport`].
pub const VALIDATE_REPORT_SCHEMA: &str = "crucible.validate_report.v1";

#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub schema_version: &'static str,
    pub spec: String,
    /// `true` iff `errors` is empty. A spec can be `valid` and still be
    /// definition-only (no `runner`) — validity is about the declared fields
    /// being honest, not about being executable.
    pub valid: bool,
    /// `true` iff the spec declares a runner and every preflight check that
    /// runner enforces passes. `false` for a definition-only spec or one that
    /// would refuse to run.
    pub runnable: bool,
    pub errors: Vec<ValidationIssue>,
    pub warnings: Vec<ValidationIssue>,
}

#[derive(Debug, Serialize)]
pub struct ValidationIssue {
    pub field: String,
    pub message: String,
}

/// Load and validate a spec file. Parse/schema failures (unknown
/// `schema_version`, malformed JSON) are returned as an `Err` — those are
/// load errors, not validation findings, matching every other `crucible`
/// subcommand's exit-1-on-load-error convention.
pub fn validate(spec_path: &Path) -> anyhow::Result<ValidationReport> {
    let spec = load_spec(spec_path)?;

    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let mut runnable = match &spec.runner {
        None => {
            warnings.push(ValidationIssue {
                field: "runner".to_string(),
                message: "definition-only spec: no runner declared, so nothing else here is checked for executability".to_string(),
            });
            false
        }
        Some(runner) => match preflight_spec(&spec, runner.kind) {
            Ok(()) => true,
            Err(err) => {
                errors.push(ValidationIssue {
                    field: "runner".to_string(),
                    message: err.to_string(),
                });
                false
            }
        },
    };

    if runnable {
        if let Some(runner) = &spec.runner {
            if runner.kind == RunnerKind::PromptBenchmark {
                if let CorpusSpec::PromptBenchmark { tasks, .. } = &runner.corpus {
                    if let Err(err) = check_prompt_regexes(tasks) {
                        errors.push(ValidationIssue {
                            field: "runner.corpus.tasks[].expectation.pattern".to_string(),
                            message: err.to_string(),
                        });
                        runnable = false;
                    }
                }
            }
        }
    }

    check_baselines(&spec, &mut warnings);
    check_portability(spec_path, &spec, &mut warnings);

    Ok(ValidationReport {
        schema_version: VALIDATE_REPORT_SCHEMA,
        spec: spec_path.display().to_string(),
        valid: errors.is_empty(),
        runnable,
        errors,
        warnings,
    })
}

/// `baselines` is genuinely unenforced today: no runner reads it. Warn, don't
/// error — refusing every spec that declares a baseline (including the real
/// flagship `pr-review-key-recall-v0.json`) would be a breaking regression
/// for a field this ticket does not yet wire into a comparison. Honest
/// reporting over either a silent lie or an unannounced breakage.
fn check_baselines(spec: &EvalSpec, warnings: &mut Vec<ValidationIssue>) {
    if !spec.baselines.is_empty() {
        warnings.push(ValidationIssue {
            field: "baselines".to_string(),
            message: format!(
                "declared baselines {:?} are not yet consumed by any runner — no baseline comparison runs",
                spec.baselines
            ),
        });
    }
}

/// A `daedalus_trials` corpus that escapes the spec's own directory tree
/// (`..` in `arena_dir`/`trials_jsonl`) only runs on a machine with the exact
/// sibling checkout at that relative path — not portable, not CI-runnable.
/// Informational, not an error: it is how the real flagship specs work today.
fn check_portability(spec_path: &Path, spec: &EvalSpec, warnings: &mut Vec<ValidationIssue>) {
    let Some(runner) = &spec.runner else {
        return;
    };
    let CorpusSpec::DaedalusTrials {
        arena_dir,
        trials_jsonl,
        ..
    } = &runner.corpus
    else {
        return;
    };
    for (field, value) in [
        ("runner.corpus.arena_dir", arena_dir),
        ("runner.corpus.trials_jsonl", trials_jsonl),
    ] {
        if Path::new(value).components().any(|c| c.as_os_str() == "..") {
            let resolved = resolve_spec_path_with_alias(spec_path, value);
            if let Some(alias) = resolved.alias {
                warnings.push(ValidationIssue {
                    field: field.to_string(),
                    message: format!(
                        "{value:?} escapes the spec's own directory tree and is not portable, but resolved here via {alias} to {}",
                        resolved.path.display()
                    ),
                });
                continue;
            }
            warnings.push(ValidationIssue {
                field: field.to_string(),
                message: format!(
                    "{value:?} escapes the spec's own directory tree — only runs on a machine with that exact sibling checkout, not portable or CI-runnable"
                ),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crucible_core::{
        AgenticJudgeConfig, AgenticJudgeTask, AggregationMethod, CorpusSpec, EvalSpec, Grader,
        GraderKind, GraderManifest, ModelProvider, PromptBenchmarkTask, PromptExpectation,
        PromptModelConfig, RunnerKind, RunnerSpec, UncertaintyRule,
    };

    fn write_spec(dir: &Path, name: &str, spec: &EvalSpec) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(path.clone(), serde_json::to_vec_pretty(spec).unwrap()).unwrap();
        path
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crucible-validate-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn base_spec() -> EvalSpec {
        EvalSpec {
            schema_version: crucible_core::EVAL_SPEC_SCHEMA.to_string(),
            id: "test".to_string(),
            task: "test".to_string(),
            inputs: String::new(),
            outputs: String::new(),
            fixtures: Vec::new(),
            graders: GraderManifest::default(),
            baselines: Vec::new(),
            aggregation: AggregationMethod::Proportion,
            uncertainty: UncertaintyRule::default(),
            decision: String::new(),
            runner: None,
        }
    }

    #[test]
    fn definition_only_spec_is_valid_but_not_runnable() {
        let dir = temp_dir("definition-only");
        let path = write_spec(&dir, "spec.json", &base_spec());
        let report = validate(&path).unwrap();
        assert!(report.valid);
        assert!(!report.runnable);
        assert!(report.errors.is_empty());
        assert!(report.warnings.iter().any(|w| w.field == "runner"));
    }

    #[test]
    fn agentic_judge_spec_without_a_declared_agentic_grader_is_invalid() {
        let dir = temp_dir("no-grader");
        let mut spec = base_spec();
        spec.runner = Some(RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: AgenticJudgeConfig {
                    provider: ModelProvider::OpenRouter,
                    model: "test/judge".to_string(),
                    judge_prompt: "Grade it.".to_string(),
                    credential_env: "OPENROUTER_API_KEY".to_string(),
                    temperature: None,
                    generator_model: None,
                    harness: None,
                    tool_allowlist: Vec::new(),
                },
                tasks: vec![AgenticJudgeTask {
                    task_id: "t1".to_string(),
                    candidate: "answer".to_string(),
                    rubric: "must be correct".to_string(),
                    expected_pass: None,
                    refuse_on_mismatch: false,
                }],
            },
        });
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(!report.valid);
        assert!(!report.runnable);
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].message.contains("Agentic grader"));
    }

    #[test]
    fn agentic_judge_spec_with_a_declared_grader_is_valid_and_runnable() {
        let dir = temp_dir("with-grader");
        let mut spec = base_spec();
        spec.graders = GraderManifest {
            graders: vec![Grader {
                id: "model-judge".to_string(),
                kind: GraderKind::Agentic,
            }],
        };
        spec.runner = Some(RunnerSpec {
            kind: RunnerKind::AgenticJudge,
            corpus: CorpusSpec::AgenticJudge {
                config: AgenticJudgeConfig {
                    provider: ModelProvider::OpenRouter,
                    model: "test/judge".to_string(),
                    judge_prompt: "Grade it.".to_string(),
                    credential_env: "OPENROUTER_API_KEY".to_string(),
                    temperature: None,
                    generator_model: None,
                    harness: None,
                    tool_allowlist: Vec::new(),
                },
                tasks: vec![AgenticJudgeTask {
                    task_id: "t1".to_string(),
                    candidate: "answer".to_string(),
                    rubric: "must be correct".to_string(),
                    expected_pass: None,
                    refuse_on_mismatch: false,
                }],
            },
        });
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(report.valid, "{:?}", report.errors);
        assert!(report.runnable);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn a_declared_confidence_other_than_0_95_is_an_error() {
        let dir = temp_dir("bad-confidence");
        let mut spec = base_spec();
        spec.graders = GraderManifest {
            graders: vec![Grader {
                id: "expected_key_match".to_string(),
                kind: GraderKind::Deterministic,
            }],
        };
        spec.uncertainty.confidence = 0.99;
        spec.runner = Some(RunnerSpec {
            kind: RunnerKind::KeyRecall,
            corpus: CorpusSpec::DaedalusTrials {
                arena_dir: "arena".to_string(),
                trials_jsonl: "trials.jsonl".to_string(),
                candidate_id: "probe".to_string(),
                tasks: Vec::new(),
            },
        });
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(!report.valid);
        assert!(!report.runnable);
        assert!(report.errors.iter().any(|e| e.message.contains("0.99")));
    }

    #[test]
    fn baselines_and_non_portable_paths_are_warnings_not_errors() {
        let dir = temp_dir("warnings");
        let mut spec = base_spec();
        spec.graders = GraderManifest {
            graders: vec![Grader {
                id: "expected_key_match".to_string(),
                kind: GraderKind::Deterministic,
            }],
        };
        spec.baselines = vec!["oracle".to_string()];
        spec.runner = Some(RunnerSpec {
            kind: RunnerKind::KeyRecall,
            corpus: CorpusSpec::DaedalusTrials {
                arena_dir: "../../daedalus/arenas/pr-review-v0".to_string(),
                trials_jsonl: "../../daedalus/runs/x/trials.jsonl".to_string(),
                candidate_id: "probe".to_string(),
                tasks: Vec::new(),
            },
        });
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(report.valid, "{:?}", report.errors);
        assert!(report.runnable, "{:?}", report.errors);
        assert!(report.warnings.iter().any(|w| w.field == "baselines"));
        assert!(report
            .warnings
            .iter()
            .any(|w| w.field == "runner.corpus.arena_dir"));
        assert!(report
            .warnings
            .iter()
            .any(|w| w.field == "runner.corpus.trials_jsonl"));
    }

    #[test]
    fn resolvable_legacy_daedalus_alias_is_a_distinct_warning() {
        let root = temp_dir("threshold-alias");
        let spec_dir = root.join("crucible/evals");
        let arena = root.join("threshold/arenas/pr-review-v0");
        let trials = root.join("threshold/runs/freeze/trials.jsonl");
        std::fs::create_dir_all(&arena).unwrap();
        std::fs::create_dir_all(trials.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&spec_dir).unwrap();
        std::fs::write(&trials, "").unwrap();

        let mut spec = base_spec();
        spec.graders = GraderManifest {
            graders: vec![Grader {
                id: "expected_key_match".to_string(),
                kind: GraderKind::Deterministic,
            }],
        };
        spec.runner = Some(RunnerSpec {
            kind: RunnerKind::KeyRecall,
            corpus: CorpusSpec::DaedalusTrials {
                arena_dir: "../../daedalus/arenas/pr-review-v0".to_string(),
                trials_jsonl: "../../daedalus/runs/freeze/trials.jsonl".to_string(),
                candidate_id: "probe".to_string(),
                tasks: Vec::new(),
            },
        });
        let path = write_spec(&spec_dir, "spec.json", &spec);
        let report = validate(&path).unwrap();

        assert!(report.valid, "{:?}", report.errors);
        assert!(report.runnable, "{:?}", report.errors);
        assert!(report.warnings.iter().any(|w| {
            w.field == "runner.corpus.arena_dir"
                && w.message
                    .contains("resolved here via daedalus_to_threshold")
                && w.message.contains("not portable")
        }));
        assert!(report.warnings.iter().any(|w| {
            w.field == "runner.corpus.trials_jsonl"
                && w.message
                    .contains("resolved here via daedalus_to_threshold")
                && w.message.contains("not portable")
        }));
    }

    #[test]
    fn a_malformed_prompt_expectation_regex_is_an_error_before_any_model_call() {
        let dir = temp_dir("bad-regex");
        let mut spec = base_spec();
        spec.graders = GraderManifest {
            graders: vec![Grader {
                id: "regex_rubric".to_string(),
                kind: GraderKind::Deterministic,
            }],
        };
        spec.runner = Some(RunnerSpec {
            kind: RunnerKind::PromptBenchmark,
            corpus: CorpusSpec::PromptBenchmark {
                config: PromptModelConfig {
                    provider: ModelProvider::OpenRouter,
                    model: "test/model".to_string(),
                    system_prompt: "Answer.".to_string(),
                    credential_env: "OPENROUTER_API_KEY".to_string(),
                    max_output_units: None,
                    temperature: None,
                    harness: None,
                    tool_allowlist: Vec::new(),
                },
                tasks: vec![PromptBenchmarkTask {
                    task_id: "broken".to_string(),
                    class: None,
                    context_file: None,
                    prompt: "irrelevant".to_string(),
                    expectation: PromptExpectation::Regex {
                        pattern: "(unclosed".to_string(),
                    },
                }],
            },
        });
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(!report.valid);
        assert!(!report.runnable);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].field,
            "runner.corpus.tasks[].expectation.pattern"
        );
        assert!(
            report.errors[0].message.contains("broken"),
            "{:?}",
            report.errors[0]
        );
    }

    #[test]
    fn unknown_schema_version_is_a_load_error_not_a_validation_finding() {
        let dir = temp_dir("bad-schema");
        let path = dir.join("spec.json");
        std::fs::write(
            &path,
            r#"{"schema_version":"crucible.eval_spec.v999","task":"x"}"#,
        )
        .unwrap();
        let err = validate(&path).expect_err("unknown schema_version must fail to load");
        assert!(err.to_string().contains("spec"));
    }
}
