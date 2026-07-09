//! A first-class **operating environment**: the model-invocation conditions an
//! eval runs *in*, declared once as data and applied to any spec.
//!
//! Crucible already carries the invocation axes an eval measures — provider,
//! model, temperature, output cap, harness, tool allowlist — but they live
//! *baked into each [`EvalSpec`]'s runner config* ([`PromptModelConfig`],
//! [`AgenticJudgeConfig`], [`HarborRunConfig`]), duplicated across the three
//! shapes. That makes "run the same eval against a different model/config" a
//! copy-the-spec-and-edit chore, and it scatters the one thing a routing
//! decision actually varies across N near-identical files.
//!
//! An [`Environment`] factors those axes out into a named, on-disk declaration.
//! Applying it is a pure [`EvalSpec`] → [`EvalSpec`] transform
//! ([`Environment::apply_to`]): each `Some` field overrides the spec's runner
//! config, each `None` leaves it untouched. The transformed spec then runs
//! through the *unchanged* execution, config-identity, and comparison stack —
//! so "run eval X in env A vs env B" is one command whose two runs land in the
//! ledger with distinct `config_id`s that differ only on the environment axis,
//! and the existing `runs compare` labels the delta `model_delta` /
//! `harness_delta` / `config_delta` by construction. The environment is the
//! *config* half of config identity, promoted from a per-spec detail to a
//! reusable declaration.
//!
//! **What an environment deliberately does not touch.** It never overrides the
//! eval's *content* — the system/judge prompt, the tasks, the rubric, the
//! fixtures. Those are what a paired comparison holds constant; letting an
//! environment change them would confound the model delta with a prompt delta
//! and make the comparison indefensible. An environment varies the *invocation*,
//! not the *question*.
//!
//! **Honest scope of `harness`/`tool_allowlist`.** For the `prompt_benchmark`
//! and `agentic_judge` runners these axes are recorded *identity*, not enforced
//! provisioning: Crucible folds them into `config_id` (so a comparison across
//! harnesses is attributable and a cross-harness delta is not silently
//! mistaken for a model delta) but it does not itself spin up the named harness
//! or sandbox the tools. Only `harbor_task` shells out to a real sandboxed
//! execution environment. Declaring these axes buys a defensible, comparable
//! identity today; it does not claim runtime enforcement Crucible does not
//! perform.

use serde::{Deserialize, Serialize};

use crate::spec::{
    AgenticJudgeConfig, CorpusSpec, EvalSpec, HarborRunConfig, ModelProvider, PromptModelConfig,
    ResourceEnvelope, RunnerKind,
};

/// Schema identifier for a persisted [`Environment`].
pub const ENVIRONMENT_SCHEMA: &str = "crucible.environment.v1";

/// A named override of an eval's model-invocation axes.
///
/// Every axis is optional: a `Some` value overrides the spec's runner config,
/// a `None` leaves the spec's value in place. This makes an environment as
/// narrow as `{ "id": "glm", "model": "z-ai/glm-4.6" }` (swap only the model,
/// hold everything else from the spec) or as wide as a full invocation
/// description. The narrow form is the routing-bench workhorse: one spec, N
/// single-axis environments, one command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// Schema identifier; defaults to [`ENVIRONMENT_SCHEMA`] for declarations
    /// that predate the field. A present value is validated on load — an
    /// unknown schema is rejected, not assumed v1.
    #[serde(
        default = "environment_schema",
        deserialize_with = "deserialize_environment_schema"
    )]
    pub schema_version: String,
    /// Stable environment id, e.g. `glm-4.6-cold` or `sonnet-5-hot`. Becomes
    /// the child output-directory name for a run made in this environment, so
    /// it must be non-empty. Free-form beyond that; humans and models read it.
    pub id: String,
    /// One-sentence description of what this environment represents. Free-form
    /// text; defaults to empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Provider adapter override. Applies to `prompt_benchmark` and
    /// `agentic_judge`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ModelProvider>,
    /// Model slug override, e.g. `openai/gpt-4o-mini`. Applies to every
    /// model-bearing runner (`prompt_benchmark`, `agentic_judge`, and
    /// `harbor_task`'s `--model`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Integer temperature override (v0 supports whole values only, matching
    /// [`PromptModelConfig::temperature`]). Applies to `prompt_benchmark` and
    /// `agentic_judge`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<u32>,
    /// Output-cap override (the spec's `max_tokens`). Applies to
    /// `prompt_benchmark` only — `agentic_judge` and `harbor_task` declare no
    /// output cap, so setting this for a spec of those kinds is an unmapped
    /// field, not a silent no-op.
    #[serde(
        rename = "max_tokens",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_output_units: Option<u32>,
    /// Credential-env override, e.g. a per-environment scoped OpenRouter key
    /// variable. Applies to `prompt_benchmark` and `agentic_judge`. Names the
    /// *env var*, never a secret value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_env: Option<String>,
    /// Harness identity override, e.g. `claude-code`. Applies to
    /// `prompt_benchmark` and `agentic_judge` as recorded identity (folded into
    /// `config_id`), not enforced provisioning — see the module docs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// Tool-allowlist override. `None` leaves the spec's list; `Some(vec![])`
    /// explicitly overrides to no tools (distinct from "unset"). Applies to
    /// `prompt_benchmark` and `agentic_judge` as recorded identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_allowlist: Option<Vec<String>>,
    /// Sandbox resource envelope override (cpu/memory/headroom). Applies to
    /// `harbor_task` only — the sole runner that runs in a real sandbox whose
    /// envelope Anthropic's Feb-2026 finding showed can move scores 6pp on its
    /// own. Setting it for a non-Harbor spec is an unmapped field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_envelope: Option<ResourceEnvelope>,
}

fn environment_schema() -> String {
    ENVIRONMENT_SCHEMA.to_string()
}

fn deserialize_environment_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    crate::serde_util::expect_schema(deserializer, ENVIRONMENT_SCHEMA)
}

/// Why applying an [`Environment`] to a spec could not produce a runnable,
/// fully-honored transformed spec.
///
/// Every variant is a refusal, not a warning: applying an environment either
/// honors every override it declares or it names exactly what it could not,
/// mirroring the rest of Crucible's "report, never silently drop" discipline
/// (cf. the promptfoo import reporting each unmappable test case).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvApplyError {
    /// The spec is definition-only — it declares no runner, so there is no
    /// model-invocation config for an environment to override.
    #[error("environment {env_id:?} cannot apply to a definition-only spec (no runner declared)")]
    DefinitionOnly {
        /// The environment whose application failed.
        env_id: String,
    },
    /// The spec's runner has no model-invocation config to override — a
    /// `key_recall` runner grades already-produced artifacts against a key and
    /// never calls a model, so there is nothing for an environment to vary.
    #[error(
        "environment {env_id:?} cannot apply to a {runner} runner: it grades produced artifacts \
         and declares no model config to override"
    )]
    NoModelConfig {
        /// The environment whose application failed.
        env_id: String,
        /// The runner kind label that carries no model config.
        runner: &'static str,
    },
    /// The environment declares one or more axes the target runner cannot
    /// accept — e.g. an output cap for an `agentic_judge` spec, or a harness
    /// for a `harbor_task` spec. Named rather than dropped so the author fixes
    /// the environment instead of trusting a run that silently ignored half of
    /// it.
    #[error(
        "environment {env_id:?} sets field(s) the {runner} runner does not accept: {}. \
         Remove them or target a runner that honors them.",
        .fields.join(", ")
    )]
    UnmappedFields {
        /// The environment whose application failed.
        env_id: String,
        /// The runner kind label that could not accept the fields.
        runner: &'static str,
        /// The environment field names that do not map onto the runner.
        fields: Vec<String>,
    },
}

/// Why an [`Environment`] declaration is not itself well-formed, independent of
/// any spec it might apply to.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvValidateError {
    /// The declaration's `id` is empty; the id names the run's output directory
    /// and the environment in receipts, so it is required.
    #[error("environment declaration has an empty id")]
    EmptyId,
    /// The declaration overrides nothing — every axis is `None`. Applying it
    /// would be a pure no-op, which is almost always an authoring mistake, so
    /// it is refused rather than silently accepted.
    #[error("environment {0:?} overrides no axes; declare at least one of provider/model/temperature/max_tokens/credential_env/harness/tool_allowlist/resource_envelope")]
    NoOverrides(String),
    /// The declaration sets an empty model slug, which would blank out the
    /// spec's model rather than override it.
    #[error("environment {0:?} sets an empty model slug")]
    EmptyModel(String),
}

impl Environment {
    /// The runner-kind label used in [`EnvApplyError`] messages, matching the
    /// wire form of [`RunnerKind`].
    fn runner_label(kind: RunnerKind) -> &'static str {
        match kind {
            RunnerKind::KeyRecall => "key_recall",
            RunnerKind::PromptBenchmark => "prompt_benchmark",
            RunnerKind::AgenticJudge => "agentic_judge",
            RunnerKind::HarborTask => "harbor_task",
        }
    }

    /// Whether this environment overrides at least one axis.
    pub fn overrides_any(&self) -> bool {
        self.provider.is_some()
            || self.model.is_some()
            || self.temperature.is_some()
            || self.max_output_units.is_some()
            || self.credential_env.is_some()
            || self.harness.is_some()
            || self.tool_allowlist.is_some()
            || self.resource_envelope.is_some()
    }

    /// Check that this declaration is well-formed on its own terms — before it
    /// touches any spec. `run --env` calls this on load so a malformed
    /// environment fails fast with a named reason, not a confusing downstream
    /// error.
    pub fn validate(&self) -> Result<(), EnvValidateError> {
        if self.id.trim().is_empty() {
            return Err(EnvValidateError::EmptyId);
        }
        if let Some(model) = &self.model {
            if model.trim().is_empty() {
                return Err(EnvValidateError::EmptyModel(self.id.clone()));
            }
        }
        if !self.overrides_any() {
            return Err(EnvValidateError::NoOverrides(self.id.clone()));
        }
        Ok(())
    }

    /// Apply this environment to `spec`, returning a new spec whose runner
    /// config carries the environment's overrides. Pure: `spec` is not
    /// mutated, and the returned spec keeps the same id, tasks, prompts,
    /// rubric, fixtures, and grader mix — only the invocation axes change.
    ///
    /// Fails ([`EnvApplyError`]) rather than silently dropping when the spec is
    /// definition-only, when the runner carries no model config, or when the
    /// environment declares an axis the target runner cannot accept.
    pub fn apply_to(&self, spec: &EvalSpec) -> Result<EvalSpec, EnvApplyError> {
        let runner = spec
            .runner
            .as_ref()
            .ok_or_else(|| EnvApplyError::DefinitionOnly {
                env_id: self.id.clone(),
            })?;
        let kind = runner.kind;
        let label = Self::runner_label(kind);

        let new_corpus = match &runner.corpus {
            CorpusSpec::PromptBenchmark { config, tasks } => {
                // prompt_benchmark honors every axis except resource_envelope.
                let mut unmapped = Vec::new();
                if self.resource_envelope.is_some() {
                    unmapped.push("resource_envelope".to_string());
                }
                self.reject_unmapped(label, unmapped)?;
                CorpusSpec::PromptBenchmark {
                    config: self.apply_prompt_config(config),
                    tasks: tasks.clone(),
                }
            }
            CorpusSpec::AgenticJudge { config, tasks } => {
                // agentic_judge has no output cap and no resource envelope.
                let mut unmapped = Vec::new();
                if self.max_output_units.is_some() {
                    unmapped.push("max_tokens".to_string());
                }
                if self.resource_envelope.is_some() {
                    unmapped.push("resource_envelope".to_string());
                }
                self.reject_unmapped(label, unmapped)?;
                CorpusSpec::AgenticJudge {
                    config: self.apply_judge_config(config),
                    tasks: tasks.clone(),
                }
            }
            CorpusSpec::HarborTasks { config, tasks } => {
                // harbor_task owns its own harness/tools/temperature; it honors
                // only the model slug and the sandbox resource envelope.
                let mut unmapped = Vec::new();
                if self.provider.is_some() {
                    unmapped.push("provider".to_string());
                }
                if self.temperature.is_some() {
                    unmapped.push("temperature".to_string());
                }
                if self.max_output_units.is_some() {
                    unmapped.push("max_tokens".to_string());
                }
                if self.credential_env.is_some() {
                    unmapped.push("credential_env".to_string());
                }
                if self.harness.is_some() {
                    unmapped.push("harness".to_string());
                }
                if self.tool_allowlist.is_some() {
                    unmapped.push("tool_allowlist".to_string());
                }
                self.reject_unmapped(label, unmapped)?;
                CorpusSpec::HarborTasks {
                    config: self.apply_harbor_config(config),
                    tasks: tasks.clone(),
                }
            }
            CorpusSpec::DaedalusTrials { .. } | CorpusSpec::CerberusReceiptBundles { .. } => {
                return Err(EnvApplyError::NoModelConfig {
                    env_id: self.id.clone(),
                    runner: label,
                });
            }
        };

        let mut out = spec.clone();
        out.runner = Some(crate::spec::RunnerSpec {
            kind,
            corpus: new_corpus,
        });
        Ok(out)
    }

    fn reject_unmapped(
        &self,
        runner: &'static str,
        fields: Vec<String>,
    ) -> Result<(), EnvApplyError> {
        if fields.is_empty() {
            Ok(())
        } else {
            Err(EnvApplyError::UnmappedFields {
                env_id: self.id.clone(),
                runner,
                fields,
            })
        }
    }

    fn apply_prompt_config(&self, config: &PromptModelConfig) -> PromptModelConfig {
        let mut config = config.clone();
        if let Some(provider) = self.provider {
            config.provider = provider;
        }
        if let Some(model) = &self.model {
            config.model = model.clone();
        }
        if let Some(temperature) = self.temperature {
            config.temperature = Some(temperature);
        }
        if let Some(max_output_units) = self.max_output_units {
            config.max_output_units = Some(max_output_units);
        }
        if let Some(credential_env) = &self.credential_env {
            config.credential_env = credential_env.clone();
        }
        if self.harness.is_some() {
            config.harness = self.harness.clone();
        }
        if let Some(tool_allowlist) = &self.tool_allowlist {
            config.tool_allowlist = tool_allowlist.clone();
        }
        config
    }

    fn apply_judge_config(&self, config: &AgenticJudgeConfig) -> AgenticJudgeConfig {
        let mut config = config.clone();
        if let Some(provider) = self.provider {
            config.provider = provider;
        }
        if let Some(model) = &self.model {
            config.model = model.clone();
        }
        if let Some(temperature) = self.temperature {
            config.temperature = Some(temperature);
        }
        if let Some(credential_env) = &self.credential_env {
            config.credential_env = credential_env.clone();
        }
        if self.harness.is_some() {
            config.harness = self.harness.clone();
        }
        if let Some(tool_allowlist) = &self.tool_allowlist {
            config.tool_allowlist = tool_allowlist.clone();
        }
        config
    }

    fn apply_harbor_config(&self, config: &HarborRunConfig) -> HarborRunConfig {
        let mut config = config.clone();
        if let Some(model) = &self.model {
            config.model = Some(model.clone());
        }
        if self.resource_envelope.is_some() {
            config.resource_envelope = self.resource_envelope;
        }
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{
        AgenticJudgeTask, CorpusSpec, GraderKind, HarborTaskSpec, PromptBenchmarkTask,
        PromptExpectation, RunnerSpec,
    };
    use crate::spec::{Grader, GraderManifest};

    fn base_spec(corpus: CorpusSpec, kind: RunnerKind) -> EvalSpec {
        EvalSpec {
            schema_version: crate::spec::EVAL_SPEC_SCHEMA.to_string(),
            id: "demo-v0".to_string(),
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
            runner: Some(RunnerSpec { kind, corpus }),
        }
    }

    fn prompt_corpus() -> CorpusSpec {
        CorpusSpec::PromptBenchmark {
            config: PromptModelConfig {
                provider: ModelProvider::OpenRouter,
                model: "openrouter/auto".to_string(),
                system_prompt: "hold-me-constant".to_string(),
                credential_env: "OPENROUTER_API_KEY".to_string(),
                max_output_units: Some(24),
                temperature: Some(0),
                harness: None,
                tool_allowlist: Vec::new(),
            },
            tasks: vec![PromptBenchmarkTask {
                task_id: "t1".to_string(),
                class: None,
                context_file: None,
                prompt: "hi".to_string(),
                expectation: PromptExpectation::Contains {
                    value: "x".to_string(),
                },
                tracked: Vec::new(),
            }],
        }
    }

    fn env(id: &str) -> Environment {
        Environment {
            schema_version: ENVIRONMENT_SCHEMA.to_string(),
            id: id.to_string(),
            description: String::new(),
            provider: None,
            model: None,
            temperature: None,
            max_output_units: None,
            credential_env: None,
            harness: None,
            tool_allowlist: None,
            resource_envelope: None,
        }
    }

    #[test]
    fn apply_overrides_prompt_model_and_holds_content_constant() {
        let spec = base_spec(prompt_corpus(), RunnerKind::PromptBenchmark);
        let e = Environment {
            model: Some("z-ai/glm-4.6".to_string()),
            temperature: Some(1),
            harness: Some("claude-code".to_string()),
            tool_allowlist: Some(vec!["bash".to_string()]),
            ..env("glm")
        };
        let out = e.apply_to(&spec).unwrap();
        let CorpusSpec::PromptBenchmark { config, tasks } = &out.runner.unwrap().corpus else {
            panic!("expected prompt_benchmark corpus");
        };
        assert_eq!(config.model, "z-ai/glm-4.6");
        assert_eq!(config.temperature, Some(1));
        assert_eq!(config.harness.as_deref(), Some("claude-code"));
        assert_eq!(config.tool_allowlist, vec!["bash".to_string()]);
        // Content held constant — the whole point of a paired comparison.
        assert_eq!(config.system_prompt, "hold-me-constant");
        assert_eq!(config.max_output_units, Some(24)); // untouched (None override)
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].prompt, "hi");
    }

    #[test]
    fn narrow_env_overrides_only_named_axis() {
        let spec = base_spec(prompt_corpus(), RunnerKind::PromptBenchmark);
        let e = Environment {
            model: Some("openai/gpt-4o-mini".to_string()),
            ..env("mini")
        };
        let out = e.apply_to(&spec).unwrap();
        let CorpusSpec::PromptBenchmark { config, .. } = &out.runner.unwrap().corpus else {
            panic!();
        };
        assert_eq!(config.model, "openai/gpt-4o-mini");
        assert_eq!(config.temperature, Some(0)); // held from spec
        assert_eq!(config.credential_env, "OPENROUTER_API_KEY");
    }

    #[test]
    fn empty_tool_allowlist_override_is_distinct_from_unset() {
        let spec = base_spec(
            CorpusSpec::PromptBenchmark {
                config: PromptModelConfig {
                    tool_allowlist: vec!["bash".to_string(), "web".to_string()],
                    ..match prompt_corpus() {
                        CorpusSpec::PromptBenchmark { config, .. } => config,
                        _ => unreachable!(),
                    }
                },
                tasks: match prompt_corpus() {
                    CorpusSpec::PromptBenchmark { tasks, .. } => tasks,
                    _ => unreachable!(),
                },
            },
            RunnerKind::PromptBenchmark,
        );
        let e = Environment {
            tool_allowlist: Some(vec![]),
            ..env("no-tools")
        };
        let out = e.apply_to(&spec).unwrap();
        let CorpusSpec::PromptBenchmark { config, .. } = &out.runner.unwrap().corpus else {
            panic!();
        };
        assert!(config.tool_allowlist.is_empty());
    }

    #[test]
    fn apply_to_key_recall_is_refused() {
        let spec = base_spec(
            CorpusSpec::DaedalusTrials {
                arena_dir: "a".to_string(),
                trials_jsonl: "t.jsonl".to_string(),
                candidate_id: "c".to_string(),
                tasks: Vec::new(),
            },
            RunnerKind::KeyRecall,
        );
        let e = Environment {
            model: Some("x/y".to_string()),
            ..env("x")
        };
        assert_eq!(
            e.apply_to(&spec).unwrap_err(),
            EnvApplyError::NoModelConfig {
                env_id: "x".to_string(),
                runner: "key_recall",
            }
        );
    }

    #[test]
    fn definition_only_spec_is_refused() {
        let mut spec = base_spec(prompt_corpus(), RunnerKind::PromptBenchmark);
        spec.runner = None;
        let e = Environment {
            model: Some("x/y".to_string()),
            ..env("x")
        };
        assert!(matches!(
            e.apply_to(&spec).unwrap_err(),
            EnvApplyError::DefinitionOnly { .. }
        ));
    }

    #[test]
    fn unmapped_output_cap_on_judge_is_named_not_dropped() {
        let spec = base_spec(
            CorpusSpec::AgenticJudge {
                config: AgenticJudgeConfig {
                    provider: ModelProvider::OpenRouter,
                    model: "anthropic/claude-opus-4".to_string(),
                    judge_prompt: "judge".to_string(),
                    credential_env: "OPENROUTER_API_KEY".to_string(),
                    temperature: Some(0),
                    generator_model: None,
                    harness: None,
                    tool_allowlist: Vec::new(),
                    format_sensitivity_check: false,
                    previous_evidence_path: None,
                },
                tasks: vec![AgenticJudgeTask {
                    task_id: "t".to_string(),
                    candidate: "c".to_string(),
                    rubric: "r".to_string(),
                    expected_pass: None,
                    refuse_on_mismatch: false,
                    reference: None,
                }],
            },
            RunnerKind::AgenticJudge,
        );
        let e = Environment {
            model: Some("openai/gpt-4o".to_string()),
            max_output_units: Some(512),
            ..env("j")
        };
        let err = e.apply_to(&spec).unwrap_err();
        assert_eq!(
            err,
            EnvApplyError::UnmappedFields {
                env_id: "j".to_string(),
                runner: "agentic_judge",
                fields: vec!["max_tokens".to_string()],
            }
        );
    }

    #[test]
    fn judge_env_without_unmapped_fields_overrides_model() {
        let spec = base_spec(
            CorpusSpec::AgenticJudge {
                config: AgenticJudgeConfig {
                    provider: ModelProvider::OpenRouter,
                    model: "anthropic/claude-opus-4".to_string(),
                    judge_prompt: "judge".to_string(),
                    credential_env: "OPENROUTER_API_KEY".to_string(),
                    temperature: Some(0),
                    generator_model: None,
                    harness: None,
                    tool_allowlist: Vec::new(),
                    format_sensitivity_check: false,
                    previous_evidence_path: None,
                },
                tasks: vec![AgenticJudgeTask {
                    task_id: "t".to_string(),
                    candidate: "c".to_string(),
                    rubric: "r".to_string(),
                    expected_pass: None,
                    refuse_on_mismatch: false,
                    reference: None,
                }],
            },
            RunnerKind::AgenticJudge,
        );
        let e = Environment {
            model: Some("openai/gpt-4o".to_string()),
            harness: Some("codex".to_string()),
            ..env("j")
        };
        let out = e.apply_to(&spec).unwrap();
        let CorpusSpec::AgenticJudge { config, .. } = &out.runner.unwrap().corpus else {
            panic!();
        };
        assert_eq!(config.model, "openai/gpt-4o");
        assert_eq!(config.harness.as_deref(), Some("codex"));
        assert_eq!(config.judge_prompt, "judge"); // content held constant
    }

    #[test]
    fn harbor_env_overrides_model_and_envelope_rejects_harness() {
        let spec = base_spec(
            CorpusSpec::HarborTasks {
                config: HarborRunConfig {
                    agent: "claude-code".to_string(),
                    model: Some("anthropic/claude-opus-4".to_string()),
                    job_timeout_ms: None,
                    resource_envelope: None,
                },
                tasks: vec![HarborTaskSpec {
                    task_id: "t".to_string(),
                    task_dir: "d".to_string(),
                }],
            },
            RunnerKind::HarborTask,
        );
        // Model + envelope map cleanly.
        let ok = Environment {
            model: Some("openai/gpt-4o".to_string()),
            resource_envelope: Some(ResourceEnvelope {
                cpu_millicores: Some(2000),
                memory_mb: Some(4096),
                headroom_percent: Some(50),
            }),
            ..env("harbor-big")
        };
        let out = ok.apply_to(&spec).unwrap();
        let CorpusSpec::HarborTasks { config, .. } = &out.runner.unwrap().corpus else {
            panic!();
        };
        assert_eq!(config.model.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(config.resource_envelope.unwrap().cpu_millicores, Some(2000));

        // harness does not map onto harbor — named, not dropped.
        let bad = Environment {
            harness: Some("claude-code".to_string()),
            ..env("harbor-h")
        };
        assert_eq!(
            bad.apply_to(&spec).unwrap_err(),
            EnvApplyError::UnmappedFields {
                env_id: "harbor-h".to_string(),
                runner: "harbor_task",
                fields: vec!["harness".to_string()],
            }
        );
    }

    #[test]
    fn validate_rejects_empty_id_no_overrides_and_empty_model() {
        assert_eq!(
            Environment {
                id: "  ".to_string(),
                model: Some("x/y".to_string()),
                ..env("")
            }
            .validate(),
            Err(EnvValidateError::EmptyId)
        );
        assert_eq!(
            env("noop").validate(),
            Err(EnvValidateError::NoOverrides("noop".to_string()))
        );
        assert_eq!(
            Environment {
                model: Some("   ".to_string()),
                ..env("blank-model")
            }
            .validate(),
            Err(EnvValidateError::EmptyModel("blank-model".to_string()))
        );
        assert!(Environment {
            model: Some("x/y".to_string()),
            ..env("good")
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn schema_round_trips_and_rejects_unknown() {
        let e = Environment {
            model: Some("x/y".to_string()),
            description: "a env".to_string(),
            ..env("rt")
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: Environment = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
        // Unknown schema is rejected on load, not coerced.
        let bad = json.replace(ENVIRONMENT_SCHEMA, "crucible.environment.v999");
        assert!(serde_json::from_str::<Environment>(&bad).is_err());
    }
}
