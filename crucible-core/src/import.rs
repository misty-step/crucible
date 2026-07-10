//! Import: project an externally-authored eval/benchmark definition into a
//! Crucible [`EvalSpec`](crate::EvalSpec).
//!
//! Crucible owns the eval artifact but does not reinvent commodity eval
//! authoring formats (`VISION.md`): rather than a bespoke one-off script per
//! external benchmark, an import adapter reads someone else's declarative
//! eval definition and projects it onto a runner Crucible already owns and
//! executes, so the imported eval runs through the same `crucible run` /
//! `validate` / `serve` pipeline as a hand-authored or `crucible author`-ed
//! spec — no shadow format.
//!
//! The first (and, for now, only) supported external format is a
//! [Promptfoo](https://promptfoo.dev)-style YAML eval config: a shared
//! prompt template, a provider, and a list of test cases each declaring
//! template `vars` plus one or more `assert`ions. This maps naturally onto
//! Crucible's `prompt_benchmark` runner, which is the only runner kind that
//! is fully self-contained (it makes its own live model call — unlike
//! `key_recall`, which grades *already-produced* candidate output, or
//! `agentic_judge`, which needs a candidate to judge). A config whose
//! provider or prompt cannot be resolved cannot produce a runnable spec at
//! all; everything else about the projection is deliberately narrow rather
//! than a generic plugin system, since this is the first real external
//! adapter and there is no second one yet to generalize from.
//!
//! **Total and honest, matching [`crate::adapter`]'s contract:** every test
//! case in the source config is accounted for in the returned
//! [`PromptfooImportReport`] — either as an imported [`PromptBenchmarkTask`]
//! or as a [`SkippedTest`] naming exactly why it could not be mapped
//! (multiple assertions, an unsupported assertion type, an unresolved
//! `$ref` template, a matrix `vars` array, or an unresolved `{{var}}`
//! placeholder). Nothing is silently dropped, and nothing is silently
//! guessed at: a test with more than one assertion is not "the first
//! assertion wins," it is reported and skipped, because Crucible's
//! `PromptBenchmarkTask` supports exactly one [`PromptExpectation`] and
//! picking one of several without saying so would misrepresent what the
//! benchmark actually checks.
//!
//! Providers and prompt templates get the same treatment one level up: a
//! config naming more than one provider or prompt is honored for its first
//! entry (one Crucible spec runs one model against one template) and every
//! other entry is named in [`PromptfooImportReport::skipped_providers`] /
//! [`PromptfooImportReport::skipped_prompts`], never silently discarded.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::spec::{PromptBenchmarkTask, PromptExpectation};

/// Raw parsed shape of a Promptfoo-style YAML eval config. Unrecognized
/// top-level keys (`outputPath`, `defaultTest`, `env`, `assertionTemplates`,
/// ...) are ignored, not rejected — Crucible reads only the subset of the
/// format it can honestly project, matching [`crate`]'s stated philosophy
/// that "unrecognized fields in real inputs are ignored, not rejected."
#[derive(Debug, Clone, Deserialize)]
pub struct PromptfooConfig {
    /// Human-readable config description, carried into the assembled spec's
    /// `inputs` field.
    #[serde(default)]
    pub description: Option<String>,
    /// Prompt template entries. Each is normally a plain string (inline
    /// template text or a `file://relative/path` reference); a non-string
    /// entry (promptfoo's object-shaped prompt form) is recognized but
    /// reported as unsupported rather than rejected at parse time.
    #[serde(default)]
    pub prompts: Vec<JsonValue>,
    /// Provider entries. Each is normally a `vendor:api-type:model` or
    /// `vendor:model` string; a non-string entry (promptfoo's object-shaped
    /// provider form, e.g. one carrying inline `config`) is recognized but
    /// reported as unsupported rather than rejected at parse time.
    #[serde(default)]
    pub providers: Vec<JsonValue>,
    /// Test cases to project into [`PromptBenchmarkTask`]s.
    #[serde(default)]
    pub tests: Vec<PromptfooTest>,
}

/// One promptfoo test case.
#[derive(Debug, Clone, Deserialize)]
pub struct PromptfooTest {
    /// Human-readable test description, used to build a stable, readable
    /// task id.
    #[serde(default)]
    pub description: Option<String>,
    /// Template variables substituted into the shared prompt template.
    #[serde(default)]
    pub vars: BTreeMap<String, JsonValue>,
    /// The assertions this test declares. Crucible imports a test only when
    /// it declares exactly one directly-mappable assertion (`equals`,
    /// `contains`, `icontains`, or `regex` with a plain scalar value).
    #[serde(default)]
    pub assert: Vec<PromptfooAssertion>,
}

/// One promptfoo assertion. Extra fields promptfoo supports (`weight`,
/// `threshold`, `metric`, ...) are ignored, not rejected.
#[derive(Debug, Clone, Deserialize)]
pub struct PromptfooAssertion {
    /// The assertion kind, e.g. `equals`, `contains`, `javascript`. Absent
    /// on a `$ref`-only entry.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// The assertion's comparison value, when it has one.
    #[serde(default)]
    pub value: Option<JsonValue>,
    /// A `$ref` template reference (promptfoo's `assertionTemplates`
    /// shorthand). Always unmappable in this pass — Crucible does not
    /// resolve the referenced template.
    #[serde(rename = "$ref", default)]
    pub reference: Option<String>,
}

/// Parse a promptfoo config from YAML (or JSON — JSON is valid YAML, so a
/// `promptfooconfig.json` parses the same way). A load/parse failure is a
/// hard error: nothing downstream can be honestly attempted without a
/// well-formed config.
pub fn parse_promptfoo_config(yaml: &str) -> Result<PromptfooConfig, serde_yaml::Error> {
    serde_yaml::from_str(yaml)
}

/// One imported test case's accounting entry when it could **not** be
/// mapped: which test, and why. `locator` names the test by index and (when
/// present) its `description`, e.g. `tests[3] ("Check if output is JSON")`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedTest {
    pub locator: String,
    pub reason: String,
}

/// The full accounting of one promptfoo import: every test in the source
/// config is either in `imported` or in `skipped_tests` — never neither.
#[derive(Debug, Clone)]
pub struct PromptfooImportReport {
    /// The OpenRouter model slug selected for the assembled spec's
    /// `prompt_benchmark` config.
    pub model: String,
    /// Which prompt source was used, e.g. `<inline>` or
    /// `file://prompts.txt (variant 1)` — informational provenance, not
    /// structural.
    pub prompt_source: String,
    /// Test cases that mapped cleanly onto a runnable Crucible task.
    pub imported: Vec<PromptBenchmarkTask>,
    /// Test cases that did **not** map cleanly, with the reason each was
    /// skipped. Total accounting: every entry in the source `tests` array
    /// not present in `imported` is here.
    pub skipped_tests: Vec<SkippedTest>,
    /// Declared providers beyond the first usable one (or an unusable one),
    /// each with a human-readable reason it was not imported.
    pub skipped_providers: Vec<String>,
    /// Declared prompt entries beyond the first usable one (or an unusable
    /// one), each with a human-readable reason it was not imported.
    pub skipped_prompts: Vec<String>,
}

/// The projection could not produce a runnable spec at all: no provider or
/// no prompt template could be resolved. Distinct from a per-test skip —
/// there is nothing to run without at least one of each.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PromptfooImportError {
    #[error("no usable provider found: {0}")]
    NoUsableProvider(String),
    #[error("no usable prompt template found: {0}")]
    NoUsablePrompt(String),
}

/// Project a parsed promptfoo config into a [`PromptfooImportReport`].
/// `base_dir` resolves `file://`-referenced prompt templates (relative to
/// the config file's own directory, matching promptfoo's own convention).
pub fn project_promptfoo(
    config: &PromptfooConfig,
    base_dir: &Path,
) -> Result<PromptfooImportReport, PromptfooImportError> {
    let provider = select_provider(config).map_err(PromptfooImportError::NoUsableProvider)?;
    let prompt = select_prompt(config, base_dir).map_err(PromptfooImportError::NoUsablePrompt)?;

    let mut imported = Vec::new();
    let mut skipped_tests = Vec::new();
    for (index, test) in config.tests.iter().enumerate() {
        let locator = test_locator(index, test);
        match project_test(index, test, &prompt.template) {
            Ok(task) => imported.push(task),
            Err(reason) => skipped_tests.push(SkippedTest { locator, reason }),
        }
    }

    Ok(PromptfooImportReport {
        model: provider.model,
        prompt_source: prompt.source,
        imported,
        skipped_tests,
        skipped_providers: provider.skipped,
        skipped_prompts: prompt.skipped,
    })
}

fn test_locator(index: usize, test: &PromptfooTest) -> String {
    match test.description.as_deref().map(str::trim) {
        Some(d) if !d.is_empty() => format!("tests[{index}] ({d:?})"),
        _ => format!("tests[{index}]"),
    }
}

/// One resolved provider selection: the model slug to run, plus every
/// declared provider that was not selected (with why).
struct SelectedProvider {
    model: String,
    skipped: Vec<String>,
}

/// Pick the first usable provider. promptfoo lets a config declare several
/// providers to test the same prompt against all of them; a single Crucible
/// `prompt_benchmark` spec runs exactly one model, so every provider beyond
/// the first usable one is reported, not silently dropped.
fn select_provider(config: &PromptfooConfig) -> Result<SelectedProvider, String> {
    let mut usable: Vec<(String, String)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for entry in &config.providers {
        match entry.as_str() {
            Some(raw) => match map_provider_slug(raw) {
                Some(slug) => usable.push((raw.to_string(), slug)),
                None => skipped.push(format!(
                    "provider {raw:?}: unrecognized vendor prefix or shape"
                )),
            },
            None => skipped.push(format!(
                "provider {entry}: non-string provider entries are not supported"
            )),
        }
    }
    if usable.is_empty() {
        return Err(if config.providers.is_empty() {
            "config declares no providers".to_string()
        } else {
            format!(
                "no usable provider among {} declared: {}",
                config.providers.len(),
                skipped.join("; ")
            )
        });
    }
    let (_selected_raw, selected_slug) = usable.remove(0);
    for (raw, slug) in usable {
        skipped.push(format!(
            "provider {raw:?} ({slug}): additional providers are not imported — one Crucible prompt_benchmark spec runs exactly one model"
        ));
    }
    Ok(SelectedProvider {
        model: selected_slug,
        skipped,
    })
}

/// Map a promptfoo provider string (`vendor:api-type:model` or
/// `vendor:model`) onto an OpenRouter `vendor/model` slug. Only a small set
/// of known vendor prefixes are recognized; anything else is refused rather
/// than guessed, per the repo's "refuse to report what it cannot defend"
/// ethos applied to model identity.
fn map_provider_slug(raw: &str) -> Option<String> {
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() < 2 {
        return None;
    }
    let vendor = parts[0];
    let model = parts[parts.len() - 1].trim();
    if model.is_empty() {
        return None;
    }
    if vendor == "openrouter" {
        // promptfoo's openrouter provider already names an OpenRouter slug
        // (often already vendor/model-shaped) after the prefix.
        return Some(model.to_string());
    }
    let mapped_vendor = match vendor {
        "openai" => "openai",
        "anthropic" => "anthropic",
        "google" | "vertex" => "google",
        "mistral" => "mistral",
        "cohere" => "cohere",
        "deepseek" => "deepseek",
        _ => return None,
    };
    Some(format!("{mapped_vendor}/{model}"))
}

/// One resolved prompt selection: the template text to run, its source
/// label, plus every declared prompt entry that was not selected (with why).
struct SelectedPrompt {
    template: String,
    source: String,
    skipped: Vec<String>,
}

/// Pick the first usable prompt template. Resolves a `file://` reference
/// relative to `base_dir`; a file carrying promptfoo's `\n---\n`-delimited
/// multi-prompt convention contributes only its first variant, with every
/// other variant reported.
fn select_prompt(config: &PromptfooConfig, base_dir: &Path) -> Result<SelectedPrompt, String> {
    // (display source label, template text, detail used only when this entry
    // ends up skipped as an "extra" — carries the actual content so a
    // skipped prompt is identifiable, not just labeled "<inline>" again).
    let mut usable: Vec<(String, String, String)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for entry in &config.prompts {
        match entry.as_str() {
            Some(raw) => match raw.strip_prefix("file://") {
                Some(rel) => resolve_prompt_file(rel, base_dir, &mut usable, &mut skipped),
                None => usable.push((
                    "<inline>".to_string(),
                    raw.to_string(),
                    format!("<inline> {raw:?}"),
                )),
            },
            None => skipped.push(format!(
                "{entry}: non-string prompt entries are not supported"
            )),
        }
    }
    if usable.is_empty() {
        return Err(if config.prompts.is_empty() {
            "config declares no prompts".to_string()
        } else {
            format!(
                "no usable prompt among {} declared: {}",
                config.prompts.len(),
                skipped.join("; ")
            )
        });
    }
    let (label, template, _) = usable.remove(0);
    for (_, _, detail) in usable {
        skipped.push(format!(
            "{detail}: additional prompts are not imported — one Crucible prompt_benchmark spec runs exactly one prompt template"
        ));
    }
    Ok(SelectedPrompt {
        template,
        source: label,
        skipped,
    })
}

fn resolve_prompt_file(
    rel: &str,
    base_dir: &Path,
    usable: &mut Vec<(String, String, String)>,
    skipped: &mut Vec<String>,
) {
    let path = base_dir.join(rel);
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let mut variants = contents.split("\n---\n");
            if let Some(first) = variants.next() {
                let label = format!("file://{rel} (variant 1)");
                usable.push((label.clone(), first.trim().to_string(), label));
            }
            for (i, _) in variants.enumerate() {
                skipped.push(format!(
                    "file://{rel} prompt variant {}: only the first prompt variant in a file is imported",
                    i + 2
                ));
            }
        }
        Err(err) => skipped.push(format!("file://{rel}: could not read ({err})")),
    }
}

/// Project one test case into a runnable [`PromptBenchmarkTask`], or a
/// human-readable reason it cannot be mapped.
fn project_test(
    index: usize,
    test: &PromptfooTest,
    template: &str,
) -> Result<PromptBenchmarkTask, String> {
    if test.assert.is_empty() {
        return Err("declares 0 assertions".to_string());
    }
    if test.assert.len() > 1 {
        return Err(format!(
            "declares {} assertions; a Crucible prompt_benchmark task supports exactly one expectation",
            test.assert.len()
        ));
    }
    if let Some((key, _)) = test.vars.iter().find(|(_, v)| v.is_array()) {
        return Err(format!(
            "vars.{key} is an array (matrix expansion) — not supported"
        ));
    }
    let expectation =
        map_assertion(&test.assert[0]).map_err(|reason| format!("assertion: {reason}"))?;
    let prompt = render_template(template, &test.vars)
        .map_err(|missing| format!("unresolved template variable {{{{{missing}}}}}"))?;

    Ok(PromptBenchmarkTask {
        task_id: task_id_for(index, test.description.as_deref()),
        class: None,
        summary: test
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string),
        context_file: None,
        prompt,
        expectation,
        tracked: Vec::new(),
    })
}

fn task_id_for(index: usize, description: Option<&str>) -> String {
    match description.map(str::trim) {
        Some(d) if !d.is_empty() => format!("test-{index}-{}", slugify(d)),
        _ => format!("test-{index}"),
    }
}

/// Lowercase-alnum-and-dash slug for a friendly, stable task id — never
/// parsed back structurally.
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
    out
}

/// Map one promptfoo assertion onto a [`PromptExpectation`], or a
/// human-readable reason it cannot be mapped. Only the assertion kinds with
/// a direct, unambiguous Crucible equivalent are supported; every other
/// kind promptfoo ships (`is-json`, `javascript`, `python`, `similar`,
/// `llm-rubric`, ...) is refused, not approximated.
fn map_assertion(assertion: &PromptfooAssertion) -> Result<PromptExpectation, String> {
    if assertion.reference.is_some() {
        return Err("uses $ref (assertionTemplates) — not supported".to_string());
    }
    let Some(kind) = assertion.kind.as_deref() else {
        return Err("has no type".to_string());
    };
    let Some(value) = &assertion.value else {
        return Err(format!("type {kind:?} has no value"));
    };
    let Some(text) = var_to_template_string(value) else {
        return Err(format!("type {kind:?} has a non-scalar value"));
    };
    match kind {
        "equals" => Ok(PromptExpectation::Exact { value: text }),
        "contains" => Ok(PromptExpectation::Contains { value: text }),
        "icontains" => Ok(PromptExpectation::CaseInsensitiveContains { value: text }),
        "regex" => Ok(PromptExpectation::Regex { pattern: text }),
        other => Err(format!("unsupported assertion type {other:?}")),
    }
}

/// Render `{{var}}` placeholders in `template` from `vars`. Fails naming the
/// first unresolved placeholder (a var neither declared nor a plain
/// string/number/bool) rather than shipping a prompt with a literal
/// `{{...}}` gap in it.
fn render_template(template: &str, vars: &BTreeMap<String, JsonValue>) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    loop {
        match rest.find("{{") {
            None => {
                out.push_str(rest);
                return Ok(out);
            }
            Some(start) => {
                out.push_str(&rest[..start]);
                let after_open = &rest[start + 2..];
                let Some(end) = after_open.find("}}") else {
                    // Unterminated `{{` — treat as literal prose, not a
                    // placeholder.
                    out.push_str("{{");
                    rest = after_open;
                    continue;
                };
                let key = after_open[..end].trim();
                match vars.get(key).and_then(var_to_template_string) {
                    Some(value) => out.push_str(&value),
                    None => return Err(key.to_string()),
                }
                rest = &after_open[end + 2..];
            }
        }
    }
}

fn var_to_template_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(s) => Some(s.clone()),
        JsonValue::Number(n) => Some(n.to_string()),
        JsonValue::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crucible-core-import-promptfoo-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn map_provider_slug_maps_known_vendors() {
        assert_eq!(
            map_provider_slug("openai:chat:gpt-5.4-mini").as_deref(),
            Some("openai/gpt-5.4-mini")
        );
        assert_eq!(
            map_provider_slug("openai:gpt-5.5").as_deref(),
            Some("openai/gpt-5.5")
        );
        assert_eq!(
            map_provider_slug("anthropic:messages:claude-sonnet-4-6").as_deref(),
            Some("anthropic/claude-sonnet-4-6")
        );
        assert_eq!(
            map_provider_slug("openrouter:z-ai/glm-5.2").as_deref(),
            Some("z-ai/glm-5.2")
        );
    }

    #[test]
    fn map_provider_slug_refuses_unknown_vendor() {
        assert_eq!(map_provider_slug("carrier-pigeon:model-x"), None);
        assert_eq!(map_provider_slug("no-colon-here"), None);
    }

    #[test]
    fn render_template_substitutes_known_vars() {
        let mut vars = BTreeMap::new();
        vars.insert("language".to_string(), json!("French"));
        vars.insert("input".to_string(), json!("Hello world"));
        let rendered = render_template(
            "Convert the following English text to {{language}}: {{input}}",
            &vars,
        )
        .unwrap();
        assert_eq!(
            rendered,
            "Convert the following English text to French: Hello world"
        );
    }

    #[test]
    fn render_template_reports_unresolved_placeholder() {
        let vars = BTreeMap::new();
        let err = render_template("Say {{greeting}}", &vars).unwrap_err();
        assert_eq!(err, "greeting");
    }

    #[test]
    fn map_assertion_supports_the_four_direct_kinds() {
        let a = |kind: &str, value: JsonValue| PromptfooAssertion {
            kind: Some(kind.to_string()),
            value: Some(value),
            reference: None,
        };
        assert_eq!(
            map_assertion(&a("equals", json!("Yarr"))).unwrap(),
            PromptExpectation::Exact {
                value: "Yarr".to_string()
            }
        );
        assert_eq!(
            map_assertion(&a("contains", json!("Bonjour"))).unwrap(),
            PromptExpectation::Contains {
                value: "Bonjour".to_string()
            }
        );
        assert_eq!(
            map_assertion(&a("icontains", json!("grub"))).unwrap(),
            PromptExpectation::CaseInsensitiveContains {
                value: "grub".to_string()
            }
        );
        assert_eq!(
            map_assertion(&a("regex", json!("^Ahoy"))).unwrap(),
            PromptExpectation::Regex {
                pattern: "^Ahoy".to_string()
            }
        );
    }

    #[test]
    fn map_assertion_refuses_unsupported_kinds_honestly() {
        for kind in ["is-json", "javascript", "python", "similar", "llm-rubric"] {
            let a = PromptfooAssertion {
                kind: Some(kind.to_string()),
                value: Some(json!("whatever")),
                reference: None,
            };
            let err = map_assertion(&a).unwrap_err();
            assert!(err.contains(kind), "{kind}: {err}");
        }
    }

    #[test]
    fn map_assertion_refuses_ref_templates() {
        let a = PromptfooAssertion {
            kind: None,
            value: None,
            reference: Some("#/assertionTemplates/x".to_string()),
        };
        let err = map_assertion(&a).unwrap_err();
        assert!(err.contains("$ref"), "{err}");
    }

    #[test]
    fn project_promptfoo_refuses_when_no_provider_is_usable() {
        let config = PromptfooConfig {
            description: None,
            prompts: vec![json!("hi")],
            providers: Vec::new(),
            tests: Vec::new(),
        };
        let err = project_promptfoo(&config, Path::new(".")).unwrap_err();
        assert_eq!(
            err,
            PromptfooImportError::NoUsableProvider("config declares no providers".to_string())
        );
    }

    #[test]
    fn project_promptfoo_refuses_when_no_prompt_is_usable() {
        let config = PromptfooConfig {
            description: None,
            prompts: Vec::new(),
            providers: vec![json!("openai:gpt-5.5")],
            tests: Vec::new(),
        };
        let err = project_promptfoo(&config, Path::new(".")).unwrap_err();
        assert_eq!(
            err,
            PromptfooImportError::NoUsablePrompt("config declares no prompts".to_string())
        );
    }

    #[test]
    fn project_promptfoo_is_total_over_a_mix_of_mappable_and_unmappable_tests() {
        let config = PromptfooConfig {
            description: Some("mini config".to_string()),
            prompts: vec![json!("Echo: {{body}}")],
            providers: vec![json!("openai:chat:gpt-5.4-mini")],
            tests: vec![
                PromptfooTest {
                    description: Some("exact match".to_string()),
                    vars: BTreeMap::from([("body".to_string(), json!("Yes"))]),
                    assert: vec![PromptfooAssertion {
                        kind: Some("equals".to_string()),
                        value: Some(json!("Echo: Yes")),
                        reference: None,
                    }],
                },
                PromptfooTest {
                    description: Some("unsupported kind".to_string()),
                    vars: BTreeMap::new(),
                    assert: vec![PromptfooAssertion {
                        kind: Some("is-json".to_string()),
                        value: None,
                        reference: None,
                    }],
                },
                PromptfooTest {
                    description: None,
                    vars: BTreeMap::new(),
                    assert: Vec::new(),
                },
            ],
        };
        let report = project_promptfoo(&config, Path::new(".")).unwrap();
        assert_eq!(report.model, "openai/gpt-5.4-mini");
        assert_eq!(report.prompt_source, "<inline>");
        assert_eq!(report.imported.len(), 1);
        assert_eq!(report.imported[0].task_id, "test-0-exact-match");
        assert_eq!(report.imported[0].prompt, "Echo: Yes");
        assert_eq!(report.skipped_tests.len(), 2);
        assert!(report.skipped_tests[0].reason.contains("is-json"));
        assert!(report.skipped_tests[1].reason.contains("0 assertions"));
    }

    #[test]
    fn project_promptfoo_reports_extra_providers_and_prompts_not_silently() {
        let config = PromptfooConfig {
            description: None,
            prompts: vec![json!("A: {{x}}"), json!("B: {{x}}")],
            providers: vec![json!("openai:gpt-5.5"), json!("anthropic:messages:claude")],
            tests: Vec::new(),
        };
        let report = project_promptfoo(&config, Path::new(".")).unwrap();
        assert_eq!(report.model, "openai/gpt-5.5");
        assert_eq!(report.skipped_providers.len(), 1);
        assert!(report.skipped_providers[0].contains("anthropic/claude"));
        assert_eq!(report.skipped_prompts.len(), 1);
        assert!(report.skipped_prompts[0].contains("B:"));
    }

    #[test]
    fn select_prompt_resolves_file_reference_and_takes_first_variant() {
        let dir = temp_dir("file-prompt");
        std::fs::write(
            dir.join("prompts.txt"),
            "First variant {{x}}\n---\nSecond variant {{x}}\n",
        )
        .unwrap();
        let config = PromptfooConfig {
            description: None,
            prompts: vec![json!("file://prompts.txt")],
            providers: vec![json!("openai:gpt-5.5")],
            tests: Vec::new(),
        };
        let report = project_promptfoo(&config, &dir).unwrap();
        assert_eq!(report.prompt_source, "file://prompts.txt (variant 1)");
        assert_eq!(report.skipped_prompts.len(), 1);
        assert!(report.skipped_prompts[0].contains("variant 2"));
    }

    #[test]
    fn real_getting_started_fixture_imports_both_clean_tests() {
        let yaml = include_str!("../tests/fixtures/promptfoo/getting-started-promptfooconfig.yaml");
        let config = parse_promptfoo_config(yaml).expect("real config must parse");
        let report = project_promptfoo(&config, Path::new(".")).expect("must be importable");
        assert_eq!(report.model, "openai/gpt-5.5");
        assert_eq!(report.imported.len(), 2, "{:?}", report.skipped_tests);
        assert!(
            report.skipped_tests.is_empty(),
            "{:?}",
            report.skipped_tests
        );
        assert!(report.imported[0].prompt.contains("French"));
        assert!(report.imported[0].prompt.contains("Hello world"));
        assert_eq!(
            report.imported[0].expectation,
            PromptExpectation::Contains {
                value: "Bonjour le monde".to_string()
            }
        );
    }

    #[test]
    fn real_simple_test_fixture_imports_two_and_reports_five_unmappable() {
        let dir = temp_dir("simple-test-real");
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/promptfoo/simple-test-prompts.txt"
            ),
            dir.join("prompts.txt"),
        )
        .unwrap();
        let yaml = include_str!("../tests/fixtures/promptfoo/simple-test-promptfooconfig.yaml");
        let config = parse_promptfoo_config(yaml).expect("real config must parse");
        let report = project_promptfoo(&config, &dir).expect("must be importable");

        assert_eq!(report.model, "openai/gpt-5.4-mini");
        assert_eq!(report.prompt_source, "file://prompts.txt (variant 1)");
        assert_eq!(report.skipped_prompts.len(), 1, "one file variant unused");

        // 5 tests total: "Check for exact match" and "Another basic substring
        // check" are cleanly mappable (equals / icontains); "Check if output
        // is JSON" (is-json), "Check for semantic similarity" (javascript +
        // python + similar -- 3 assertions), and "Use LLM to evaluate output"
        // ($ref + llm-rubric -- 2 assertions) are not.
        assert_eq!(report.imported.len(), 2, "{:?}", report.skipped_tests);
        assert_eq!(report.skipped_tests.len(), 3, "{:?}", report.skipped_tests);
        assert!(
            report
                .skipped_tests
                .iter()
                .any(|s| s.locator.contains("Check if output is JSON")
                    && s.reason.contains("is-json"))
        );
        assert!(report
            .skipped_tests
            .iter()
            .any(|s| s.locator.contains("Check for semantic similarity")
                && s.reason.contains("supports exactly one expectation")));
        assert!(report
            .skipped_tests
            .iter()
            .any(|s| s.locator.contains("Use LLM to evaluate output")
                && s.reason.contains("supports exactly one expectation")));

        let exact = report
            .imported
            .iter()
            .find(|t| t.task_id.contains("exact-match"))
            .expect("exact match task imported");
        assert_eq!(
            exact.expectation,
            PromptExpectation::Exact {
                value: "Yarr".to_string()
            }
        );
        assert!(exact
            .prompt
            .contains("Rephrase this from English to Pirate: Yes"));
    }
}
