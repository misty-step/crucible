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
    check_prompt_regexes, check_prompt_tracked_ids, load_spec, preflight_spec,
    resolve_spec_path_with_alias,
};
use crucible_core::{required_sample_size, CorpusSpec, EvalSpec, RunnerKind};

/// Schema identifier for a persisted [`ValidationReport`].
pub const VALIDATE_REPORT_SCHEMA: &str = "crucible.validate_report.v1";

#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub schema_version: &'static str,
    pub spec: String,
    /// The spec's declared display `title` (operator UX ruling 2026-07-09),
    /// `None` when the spec predates the field or never set one. Purely
    /// presentational — carried through so a cold agent or the serve UI
    /// never has to re-parse the spec file just to show a human name
    /// alongside its validation result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
                    if let Err(err) = check_prompt_tracked_ids(tasks) {
                        errors.push(ValidationIssue {
                            field: "runner.corpus.tasks[].tracked[].id".to_string(),
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
    check_power(&spec, &mut warnings);

    let title = spec.title.clone();
    Ok(ValidationReport {
        schema_version: VALIDATE_REPORT_SCHEMA,
        spec: spec_path.display().to_string(),
        title,
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

/// Significance/power targets `check_power` warns against — Kotawala's own
/// convention (*Resolution Diagnostics for Paired LLM Evaluation*,
/// arXiv:2605.30315), and the same `(alpha, power)` pair
/// `run_store::RESOLUTION_TARGET_POWER`/`DEFAULT_ALPHA` use for the
/// retrospective `runs compare` diagnostic — so a spec's prospective and
/// retrospective power checks agree on what "adequately powered" means.
const POWER_CHECK_ALPHA: f64 = 0.05;
const POWER_CHECK_POWER: f64 = 0.8;

/// Conservative (worst-case) baseline proportion for `check_power`'s
/// one-sample proxy: `p(1-p)` is maximized at `p = 0.5`, the largest
/// variance — and therefore the largest required-N — any Bernoulli baseline
/// could have. Validate time has no paired discordance data yet (the eval
/// has not run), so there is no better baseline estimate to plug in; `0.5`
/// guarantees this check never under-warns by picking an optimistic
/// baseline no data supports.
const POWER_CHECK_CONSERVATIVE_BASELINE: f64 = 0.5;

/// Warn when the spec declares `min_effect_of_interest` but its own declared
/// task count cannot resolve that effect at `(alpha=0.05, power=0.8)`.
///
/// This is deliberately the SIMPLE one-sample [`required_sample_size`] proxy,
/// not the correct paired-Bernoulli formula `runs compare` uses
/// ([`crucible_core::required_n_paired`]): that formula needs real observed
/// discordant counts (`b`, `c`) from an actual paired run, which does not
/// exist yet at validate time. A conservative prospective sanity check
/// ("this many tasks probably can't resolve this effect") is honest about
/// what it is; a fabricated paired estimate from data that doesn't exist
/// would not be.
fn check_power(spec: &EvalSpec, warnings: &mut Vec<ValidationIssue>) {
    let Some(min_effect) = spec.min_effect_of_interest else {
        return;
    };
    let Some(runner) = &spec.runner else {
        return;
    };
    let Some(declared_n) = declared_task_count(&runner.corpus) else {
        return;
    };
    let Some(required_n) = required_sample_size(
        POWER_CHECK_CONSERVATIVE_BASELINE,
        min_effect,
        POWER_CHECK_ALPHA,
        POWER_CHECK_POWER,
    ) else {
        return;
    };
    if declared_n < required_n {
        warnings.push(ValidationIssue {
            field: "min_effect_of_interest".to_string(),
            message: format!(
                "declared {declared_n} task(s) cannot resolve an effect of {min_effect} at \
                 (alpha={POWER_CHECK_ALPHA}, power={POWER_CHECK_POWER}) — at least {required_n} \
                 would be needed (conservative one-sample proxy at a worst-case baseline of \
                 {POWER_CHECK_CONSERVATIVE_BASELINE}); see docs/design-references.md §1 \
                 (Kotawala, arXiv:2605.30315) for why an underpowered comparison can look \
                 identical to a genuine \"no difference\" verdict"
            ),
        });
    }
}

/// The task/trial count a corpus declares, when it is knowable from the
/// spec alone. `None` for [`CorpusSpec::DaedalusTrials`]: its `tasks` field
/// is an ALLOWLIST (empty means "every trial in the referenced file"), so
/// neither its length nor zero is the honest executed count without reading
/// that external file — which validate deliberately avoids for portability
/// reasons (see [`check_portability`]).
fn declared_task_count(corpus: &CorpusSpec) -> Option<u64> {
    match corpus {
        CorpusSpec::DaedalusTrials { .. } => None,
        CorpusSpec::CerberusReceiptBundles { tasks, .. } => Some(tasks.len() as u64),
        CorpusSpec::PromptBenchmark { tasks, .. } => Some(tasks.len() as u64),
        CorpusSpec::AgenticJudge { tasks, .. } => Some(tasks.len() as u64),
        CorpusSpec::HarborTasks { tasks, .. } => Some(tasks.len() as u64),
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
        PromptModelConfig, RunnerKind, RunnerSpec, TrackedCheck, UncertaintyRule,
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
            title: None,
            context: None,
            task: "test".to_string(),
            inputs: String::new(),
            outputs: String::new(),
            fixtures: Vec::new(),
            graders: GraderManifest::default(),
            baselines: Vec::new(),
            aggregation: AggregationMethod::Proportion,
            uncertainty: UncertaintyRule::default(),
            decision: String::new(),
            min_effect_of_interest: None,
            runner: None,
        }
    }

    #[test]
    fn validate_report_carries_the_spec_declared_title_when_present() {
        let dir = temp_dir("titled");
        let mut spec = base_spec();
        spec.title = Some("Test eval, human name".to_string());
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert_eq!(report.title.as_deref(), Some("Test eval, human name"));
    }

    #[test]
    fn validate_report_title_is_none_when_the_spec_declares_none() {
        let dir = temp_dir("untitled");
        let path = write_spec(&dir, "spec.json", &base_spec());
        let report = validate(&path).unwrap();
        assert!(report.title.is_none());
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
                    format_sensitivity_check: false,
                    previous_evidence_path: None,
                },
                tasks: vec![AgenticJudgeTask {
                    task_id: "t1".to_string(),
                    candidate: "answer".to_string(),
                    rubric: "must be correct".to_string(),
                    expected_pass: None,
                    refuse_on_mismatch: false,
                    reference: None,
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
                    format_sensitivity_check: false,
                    previous_evidence_path: None,
                },
                tasks: vec![AgenticJudgeTask {
                    task_id: "t1".to_string(),
                    candidate: "answer".to_string(),
                    rubric: "must be correct".to_string(),
                    expected_pass: None,
                    refuse_on_mismatch: false,
                    reference: None,
                }],
            },
        });
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(report.valid, "{:?}", report.errors);
        assert!(report.runnable);
        assert!(report.errors.is_empty());
    }

    fn agentic_judge_spec_with_n_tasks(n: usize) -> EvalSpec {
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
                    format_sensitivity_check: false,
                    previous_evidence_path: None,
                },
                tasks: (0..n)
                    .map(|i| AgenticJudgeTask {
                        task_id: format!("t{i}"),
                        candidate: "answer".to_string(),
                        rubric: "must be correct".to_string(),
                        expected_pass: None,
                        refuse_on_mismatch: false,
                        reference: None,
                    })
                    .collect(),
            },
        });
        spec
    }

    #[test]
    fn check_power_warns_when_declared_tasks_cannot_resolve_the_effect() {
        let dir = temp_dir("power-underpowered");
        let mut spec = agentic_judge_spec_with_n_tasks(2);
        // At a conservative 0.5 baseline, resolving a 0.05 effect at
        // (alpha=0.05, power=0.8) needs ~783 tasks — 2 is nowhere close.
        spec.min_effect_of_interest = Some(0.05);
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(
            report.valid,
            "a power warning is non-fatal: {:?}",
            report.errors
        );
        let warning = report
            .warnings
            .iter()
            .find(|w| w.field == "min_effect_of_interest")
            .expect("declared task count cannot resolve the effect");
        assert!(warning.message.contains("2 task"), "{}", warning.message);
        assert!(warning.message.contains("0.05"), "{}", warning.message);
        assert!(
            warning.message.contains("arXiv:2605.30315"),
            "{}",
            warning.message
        );
    }

    #[test]
    fn check_power_is_silent_when_declared_tasks_can_resolve_the_effect() {
        let dir = temp_dir("power-adequate");
        let mut spec = agentic_judge_spec_with_n_tasks(10);
        // At a conservative 0.5 baseline, resolving a 0.45 effect at
        // (alpha=0.05, power=0.8) needs only 7 tasks — 10 clears it.
        spec.min_effect_of_interest = Some(0.45);
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(report.valid, "{:?}", report.errors);
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| w.field == "min_effect_of_interest"),
            "declared task count adequately resolves the effect: {:?}",
            report.warnings
        );
    }

    #[test]
    fn check_power_is_silent_when_no_effect_of_interest_is_declared() {
        let dir = temp_dir("power-undeclared");
        let spec = agentic_judge_spec_with_n_tasks(1); // adequate for nothing, but nothing was asked
        assert_eq!(spec.min_effect_of_interest, None);
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(report.valid, "{:?}", report.errors);
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| w.field == "min_effect_of_interest"),
            "no effect declared, nothing to check: {:?}",
            report.warnings
        );
    }

    #[test]
    fn declared_task_count_is_none_for_a_daedalus_trials_allowlist_corpus() {
        // The `tasks` field on a DaedalusTrials corpus is an allowlist, not
        // an executed count — empty means "every trial in the file", so
        // neither its length nor zero is the honest count without reading
        // that external file.
        let corpus = CorpusSpec::DaedalusTrials {
            arena_dir: "arena".to_string(),
            trials_jsonl: "trials.jsonl".to_string(),
            candidate_id: "candidate".to_string(),
            tasks: Vec::new(),
        };
        assert_eq!(declared_task_count(&corpus), None);
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
                    request_timeout_seconds: None,
                },
                tasks: vec![PromptBenchmarkTask {
                    task_id: "broken".to_string(),
                    class: None,
                    summary: None,
                    context_file: None,
                    prompt: "irrelevant".to_string(),
                    expectation: PromptExpectation::Regex {
                        pattern: "(unclosed".to_string(),
                    },
                    tracked: Vec::new(),
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

    fn prompt_benchmark_spec_with_tracked(ids: &[&str]) -> EvalSpec {
        let mut spec = base_spec();
        spec.graders = GraderManifest {
            graders: vec![Grader {
                id: "text_rubric".to_string(),
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
                    request_timeout_seconds: None,
                },
                tasks: vec![PromptBenchmarkTask {
                    task_id: "tracked-task".to_string(),
                    class: None,
                    summary: None,
                    context_file: None,
                    prompt: "Say crucible-smoke.".to_string(),
                    expectation: PromptExpectation::Contains {
                        value: "crucible".to_string(),
                    },
                    tracked: ids
                        .iter()
                        .map(|id| TrackedCheck {
                            id: (*id).to_string(),
                            expectation: PromptExpectation::Contains {
                                value: "smoke".to_string(),
                            },
                        })
                        .collect(),
                }],
            },
        });
        spec
    }

    #[test]
    fn prompt_benchmark_tracked_checks_validate_cleanly() {
        let dir = temp_dir("tracked-valid");
        let spec = prompt_benchmark_spec_with_tracked(&["mentions-smoke", "mentions-crucible"]);
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(report.valid, "{:?}", report.errors);
        assert!(report.runnable, "{:?}", report.errors);
    }

    #[test]
    fn duplicate_tracked_id_within_one_task_is_invalid() {
        let dir = temp_dir("tracked-duplicate");
        let spec = prompt_benchmark_spec_with_tracked(&["style", "style"]);
        let path = write_spec(&dir, "spec.json", &spec);
        let report = validate(&path).unwrap();
        assert!(!report.valid);
        assert!(!report.runnable);
        let error = report
            .errors
            .iter()
            .find(|error| error.field == "runner.corpus.tasks[].tracked[].id")
            .expect("duplicate tracked id is reported");
        assert!(
            error.message.contains("tracked-task") && error.message.contains("style"),
            "{error:?}"
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
