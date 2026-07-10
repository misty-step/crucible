//! `crucible publish` (crucible-publish-packet): export one persisted run as
//! a self-contained public benchmark packet, `crucible.bench_packet.v1`.
//!
//! This is the ONLY door between the private run ledger (which may carry
//! proprietary diffs and API-keyed transcripts) and a public benchmark site
//! — refusal semantics matter more than features. `publish` never writes to
//! the ledger; it only reads a [`crate::run_store::StoredRun`] plus its
//! evidence and (optional) spec files, cross-checks that evidence actually
//! belongs to the run record it is attached to, and refuses (never emits a
//! partial packet) on any of:
//!
//! - an unknown run id ([`crate::run_store::show_run`]'s own not-found error)
//! - any runner kind other than `prompt_benchmark` (v1's only publishable kind)
//! - a missing or unreadable evidence file
//! - an evidence benchmark/model that disagrees with the run record
//! - an evidence task id with no matching declared task in the spec, or vice
//!   versa (the join is exhaustive both directions)
//!
//! A missing *spec* file (as opposed to missing evidence) is tolerated: the
//! spec-derived fields (`title`, `decision`, `config.system_prompt`, and each
//! task's `prompt`/`summary`/`expectation`) are emitted `null` and a note is
//! printed to stderr, since the evidence alone already carries every
//! ledger-owned fact (score, tokens, cost, pass/fail, raw output).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use crucible_core::{CorpusSpec, EvalSpec, PromptBenchmarkTask};
use serde::Serialize;
use serde_json::Value;

use crate::run_store;
use crate::serve::expectation_kind_and_value;
use crate::spec_run;
use crate::spec_save::slugify;

pub const BENCH_PACKET_SCHEMA: &str = "crucible.bench_packet.v1";

#[derive(Debug, Serialize)]
struct BenchPacket {
    schema_version: &'static str,
    benchmark: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision: Option<String>,
    run_id: String,
    executed_at_unix_ms: i64,
    provenance: PacketProvenance,
    config: PacketConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_model: Option<String>,
    trusted: bool,
    score: PacketScore,
    classes: Vec<PacketClass>,
    totals: PacketTotals,
    tasks: Vec<PacketTask>,
}

#[derive(Debug, Serialize)]
struct PacketProvenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_sha: Option<String>,
}

#[derive(Debug, Serialize)]
struct PacketConfig {
    config_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_allowlist: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct PacketScore {
    successes: u64,
    n: u64,
    point: Option<f64>,
    lower: f64,
    upper: f64,
    method: String,
    confidence: f64,
}

#[derive(Debug, Serialize)]
struct PacketClass {
    class: String,
    successes: u64,
    n: u64,
}

#[derive(Debug, Serialize)]
struct PacketTotals {
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
    /// Summed per-task wall time (crucible-http-timeout-config: "how long a
    /// model will go" is data, not infrastructure residue). Absent when the
    /// evidence predates `latency_ms`.
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct PacketExpectation {
    kind: String,
    value: Value,
}

#[derive(Debug, Serialize)]
struct PacketTask {
    task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expectation: Option<PacketExpectation>,
    output: String,
    passed: bool,
    /// Per-task wall time from the evidence's `latency_ms`; absent for
    /// pre-duration evidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
}

/// Publish `run_id` from `db` as a `crucible.bench_packet.v1` JSON file under
/// `out`, returning the path written. See the module doc for the exact
/// refusal contract.
pub fn publish(run_id: &str, db: &Path, out: &Path) -> anyhow::Result<PathBuf> {
    let detail =
        run_store::show_run(db, run_id).with_context(|| format!("looking up run {run_id:?}"))?;
    let run = &detail.run;

    if run.runner_kind != "prompt_benchmark" {
        anyhow::bail!(
            "refusing to publish run {run_id:?}: runner kind {:?} is not yet publishable \
             (crucible publish only supports prompt_benchmark runs in v1)",
            run.runner_kind
        );
    }

    let evidence_path = run.evidence_path.as_deref().with_context(|| {
        format!("refusing to publish run {run_id:?}: no evidence artifact is recorded for it")
    })?;
    let evidence_bytes = std::fs::read(evidence_path).with_context(|| {
        format!("refusing to publish run {run_id:?}: could not read evidence file {evidence_path}")
    })?;
    let evidence: Value = serde_json::from_slice(&evidence_bytes).with_context(|| {
        format!(
            "refusing to publish run {run_id:?}: evidence file {evidence_path} is not valid JSON"
        )
    })?;

    let evidence_benchmark = evidence
        .get("spec_id")
        .and_then(Value::as_str)
        .with_context(|| {
            format!(
                "refusing to publish run {run_id:?}: evidence file {evidence_path} is missing spec_id"
            )
        })?;
    if evidence_benchmark != run.benchmark_id {
        anyhow::bail!(
            "refusing to publish run {run_id:?}: evidence benchmark {evidence_benchmark:?} \
             disagrees with the run record's benchmark {:?}",
            run.benchmark_id
        );
    }
    let evidence_model = evidence
        .get("model")
        .and_then(Value::as_str)
        .with_context(|| {
            format!(
                "refusing to publish run {run_id:?}: evidence file {evidence_path} is missing model"
            )
        })?;
    if Some(evidence_model) != run.model.as_deref() {
        anyhow::bail!(
            "refusing to publish run {run_id:?}: evidence model {evidence_model:?} disagrees \
             with the run record's model {:?}",
            run.model
        );
    }

    let evidence_tasks = evidence
        .get("tasks")
        .and_then(Value::as_array)
        .with_context(|| {
            format!(
                "refusing to publish run {run_id:?}: evidence file {evidence_path} has no tasks array"
            )
        })?;

    let spec = load_spec_for_publish(run.spec_path.as_deref(), run_id);
    let title = spec.as_ref().and_then(|spec| spec.title.clone());
    let decision = spec
        .as_ref()
        .map(|spec| spec.decision.clone())
        .filter(|decision| !decision.is_empty());
    let prompt_corpus = spec.as_ref().and_then(spec_prompt_corpus);
    let system_prompt = prompt_corpus.map(|(config, _)| config.system_prompt.clone());
    let spec_tasks: Option<BTreeMap<&str, &PromptBenchmarkTask>> =
        prompt_corpus.map(|(_, tasks)| {
            tasks
                .iter()
                .map(|task| (task.task_id.as_str(), task))
                .collect()
        });

    let mut classes: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut total_tokens: Option<u64> = None;
    let mut total_cost_usd: Option<f64> = None;
    let mut total_duration_ms: Option<u64> = None;
    let mut consumed_spec_task_ids: HashSet<&str> = HashSet::new();
    let mut tasks = Vec::with_capacity(evidence_tasks.len());

    for task in evidence_tasks {
        let task_id = task
            .get("task_id")
            .and_then(Value::as_str)
            .with_context(|| {
                format!(
                    "refusing to publish run {run_id:?}: a task in {evidence_path} is missing task_id"
                )
            })?
            .to_string();
        let class = task
            .get("class")
            .and_then(Value::as_str)
            .map(str::to_string);
        let output = task
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let passed = task.get("passed").and_then(Value::as_bool).with_context(|| {
            format!(
                "refusing to publish run {run_id:?}: task {task_id:?} in {evidence_path} is missing passed"
            )
        })?;
        let tokens = task.get("total_tokens").and_then(Value::as_u64);
        let cost_usd = task.get("cost_usd").and_then(Value::as_f64);
        let duration_ms = task.get("latency_ms").and_then(Value::as_u64);

        if let Some(tokens) = tokens {
            total_tokens = Some(total_tokens.unwrap_or(0) + tokens);
        }
        if let Some(cost_usd) = cost_usd {
            total_cost_usd = Some(total_cost_usd.unwrap_or(0.0) + cost_usd);
        }
        if let Some(duration_ms) = duration_ms {
            total_duration_ms = Some(total_duration_ms.unwrap_or(0) + duration_ms);
        }
        if let Some(class) = &class {
            let entry = classes.entry(class.clone()).or_insert((0, 0));
            entry.1 += 1;
            if passed {
                entry.0 += 1;
            }
        }

        let (summary, prompt, expectation) = match &spec_tasks {
            Some(map) => {
                let spec_task = map.get(task_id.as_str()).with_context(|| {
                    format!(
                        "refusing to publish run {run_id:?}: evidence task {task_id:?} has no \
                         matching task in the spec's declared tasks"
                    )
                })?;
                consumed_spec_task_ids.insert(spec_task.task_id.as_str());
                let (kind, value) = expectation_kind_and_value(&spec_task.expectation);
                (
                    spec_task.summary.clone(),
                    Some(spec_task.prompt.clone()),
                    Some(PacketExpectation { kind, value }),
                )
            }
            None => (None, None, None),
        };

        tasks.push(PacketTask {
            task_id,
            class,
            summary,
            prompt,
            expectation,
            output,
            passed,
            duration_ms,
            tokens,
            cost_usd,
        });
    }

    if let Some(map) = &spec_tasks {
        if consumed_spec_task_ids.len() != map.len() {
            let mut missing: Vec<&str> = map
                .keys()
                .filter(|task_id| !consumed_spec_task_ids.contains(*task_id))
                .copied()
                .collect();
            missing.sort_unstable();
            anyhow::bail!(
                "refusing to publish run {run_id:?}: the spec declares task(s) {missing:?} with \
                 no matching evidence"
            );
        }
    }

    let provider = evidence
        .get("provider")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| run.provider.clone());
    let temperature = evidence
        .get("temperature")
        .and_then(Value::as_u64)
        .map(|value| value as u32);
    let max_tokens = evidence
        .get("max_output_units")
        .and_then(Value::as_u64)
        .map(|value| value as u32);
    let harness = evidence
        .get("harness")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| run.harness.clone());
    let tool_allowlist = evidence
        .get("tool_allowlist")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .or_else(|| (!run.tool_allowlist.is_empty()).then(|| run.tool_allowlist.clone()));

    let response_model = (!run.response_model.is_empty()).then(|| run.response_model.clone());

    let packet = BenchPacket {
        schema_version: BENCH_PACKET_SCHEMA,
        benchmark: run.benchmark_id.clone(),
        title,
        decision,
        run_id: run.run_id.clone(),
        executed_at_unix_ms: run.created_at_unix_ms,
        provenance: PacketProvenance {
            repo: run.repo.clone(),
            git_sha: run.git_sha.clone(),
        },
        config: PacketConfig {
            config_id: run.config_id.clone(),
            provider,
            model: evidence_model.to_string(),
            temperature,
            max_tokens,
            system_prompt,
            harness,
            tool_allowlist,
        },
        response_model,
        trusted: run.trusted,
        score: PacketScore {
            successes: run.successes,
            n: run.n,
            point: run.point,
            lower: run.lower,
            upper: run.upper,
            method: run.method.clone(),
            confidence: run.confidence,
        },
        classes: classes
            .into_iter()
            .map(|(class, (successes, n))| PacketClass {
                class,
                successes,
                n,
            })
            .collect(),
        totals: PacketTotals {
            tokens: total_tokens,
            cost_usd: total_cost_usd,
            duration_ms: total_duration_ms,
        },
        tasks,
    };

    std::fs::create_dir_all(out)
        .with_context(|| format!("creating output directory {}", out.display()))?;
    let filename = format!(
        "{}--{}--{}.json",
        slugify(&run.benchmark_id),
        slugify(evidence_model),
        short_run_id(&run.run_id)
    );
    let packet_path = out.join(filename);
    let json = serde_json::to_string_pretty(&packet).context("serializing bench packet")?;
    std::fs::write(&packet_path, format!("{json}\n"))
        .with_context(|| format!("writing bench packet {}", packet_path.display()))?;
    Ok(packet_path)
}

/// Load the run's spec when `spec_path` is recorded, tolerating absence or a
/// load failure by returning `None` and printing why to stderr — the spec is
/// definition data that enriches the packet (title/decision/system prompt/
/// per-task prompt+summary+expectation); its absence never blocks publishing
/// the evidence-owned facts (score, tokens, cost, pass/fail, raw output).
fn load_spec_for_publish(spec_path: Option<&str>, run_id: &str) -> Option<EvalSpec> {
    let Some(path) = spec_path else {
        eprintln!(
            "note: run {run_id:?} has no spec_path recorded; publishing without spec-derived \
             fields (title, decision, config.system_prompt, task prompt/summary/expectation)"
        );
        return None;
    };
    match spec_run::load_spec(Path::new(path)) {
        Ok(spec) => Some(spec),
        Err(err) => {
            eprintln!(
                "warning: run {run_id:?} could not load spec {path:?} ({err:#}); publishing \
                 without spec-derived fields (title, decision, config.system_prompt, task \
                 prompt/summary/expectation)"
            );
            None
        }
    }
}

fn spec_prompt_corpus(
    spec: &EvalSpec,
) -> Option<(&crucible_core::PromptModelConfig, &Vec<PromptBenchmarkTask>)> {
    let runner = spec.runner.as_ref()?;
    match &runner.corpus {
        CorpusSpec::PromptBenchmark { config, tasks } => Some((config, tasks)),
        _ => None,
    }
}

/// A short, filename-safe disambiguator derived from a run id — not a stable
/// hash across Rust versions/compilations, just enough to keep repeated
/// publishes of the same benchmark+model from colliding in one output
/// directory.
fn short_run_id(run_id: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    run_id.hash(&mut hasher);
    format!("{:016x}", hasher.finish())[..10].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval_run::{EvalReport, RunReport, Score, RUN_REPORT_SCHEMA};

    const BENCHMARK: &str = "publish-fixture-v0";
    const MODEL: &str = "test/publish-model";

    fn temp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crucible-publish-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// A full, joinable spec: two prompt tasks with class/summary/prompt/
    /// expectation matching the evidence fixture's task ids exactly.
    fn write_spec(root: &Path, credential_env: &str) -> PathBuf {
        let spec_path = root.join(format!("{BENCHMARK}.json"));
        let spec_json = serde_json::json!({
            "schema_version": "crucible.eval_spec.v1",
            "id": BENCHMARK,
            "title": "Publish Fixture Benchmark",
            "task": "publish-fixture",
            "decision": "Ship the packet if the pass rate holds.",
            "runner": {
                "kind": "prompt_benchmark",
                "corpus": {
                    "source": "prompt_benchmark",
                    "config": {
                        "provider": "open_router",
                        "model": MODEL,
                        "system_prompt": "Answer exactly.",
                        "credential_env": credential_env
                    },
                    "tasks": [
                        {
                            "task_id": "t0",
                            "class": "format_adherence",
                            "summary": "recall the exact key",
                            "prompt": "What is the key?",
                            "expectation": { "kind": "exact", "value": "match" }
                        },
                        {
                            "task_id": "t1",
                            "class": "format_adherence",
                            "summary": "recall a different key",
                            "prompt": "What is the other key?",
                            "expectation": { "kind": "exact", "value": "match" }
                        }
                    ]
                }
            }
        });
        std::fs::write(
            &spec_path,
            serde_json::to_string_pretty(&spec_json).unwrap(),
        )
        .expect("write spec fixture");
        spec_path
    }

    fn write_evidence(spec_path: &Path, out: &Path) -> PathBuf {
        let evidence = serde_json::json!({
            "schema_version": "crucible.prompt_run_evidence.v1",
            "spec_id": BENCHMARK,
            "spec": spec_path.display().to_string(),
            "runner": "prompt_benchmark",
            "provider": "open_router",
            "model": MODEL,
            "temperature": 0,
            "max_output_units": 8,
            "system_prompt_hash": "fnv1a64:test",
            "score": {
                "metric": "prompt_rubric_pass_rate",
                "successes": 1,
                "n": 2,
                "point": 0.5,
                "lower": 0.01,
                "upper": 0.99,
                "confidence": 0.95,
                "method": "Wilson"
            },
            "totals": { "tasks": 2, "passed": 1, "failed": 1 },
            "tasks": [
                {
                    "task_id": "t0",
                    "class": "format_adherence",
                    "prompt_hash": "fnv1a64:prompt-0",
                    "rubric_hash": "fnv1a64:rubric-0",
                    "expectation": { "kind": "exact", "value": "match" },
                    "passed": true,
                    "output": "match",
                    "latency_ms": 12,
                    "requested_model": MODEL,
                    "response_model": MODEL,
                    "prompt_tokens": 5,
                    "completion_tokens": 2,
                    "total_tokens": 7,
                    "cost_usd": 0.001
                },
                {
                    "task_id": "t1",
                    "class": "format_adherence",
                    "prompt_hash": "fnv1a64:prompt-1",
                    "rubric_hash": "fnv1a64:rubric-1",
                    "expectation": { "kind": "exact", "value": "match" },
                    "passed": false,
                    "output": "miss",
                    "latency_ms": 9,
                    "requested_model": MODEL,
                    "response_model": MODEL,
                    "prompt_tokens": 4,
                    "completion_tokens": 3,
                    "total_tokens": 7,
                    "cost_usd": 0.001
                }
            ]
        });
        let evidence_path = out.join("prompt-run.json");
        std::fs::write(
            &evidence_path,
            format!("{}\n", serde_json::to_string_pretty(&evidence).unwrap()),
        )
        .expect("write evidence fixture");
        evidence_path
    }

    /// Persists a full, real (spec + evidence + run record) prompt_benchmark
    /// run under `root`, returning the assigned run id.
    fn seed_run(root: &Path, credential_env: &str) -> (PathBuf, String) {
        let out = root.join("out");
        std::fs::create_dir_all(&out).expect("create output dir");
        let spec_path = write_spec(root, credential_env);
        let evidence_path = write_evidence(&spec_path, &out);
        let report = RunReport {
            schema_version: RUN_REPORT_SCHEMA,
            output_dir: out.display().to_string(),
            evals: vec![EvalReport {
                id: BENCHMARK.to_string(),
                title: "Publish fixture".to_string(),
                score: Score {
                    metric: "prompt_rubric_pass_rate",
                    successes: 1,
                    n: 2,
                    point: Some(0.5),
                    lower: 0.01,
                    upper: 0.99,
                    confidence: 0.95,
                    method: "Wilson",
                },
                artifacts: vec![
                    spec_path.display().to_string(),
                    evidence_path.display().to_string(),
                ],
                notes: Vec::new(),
            }],
        };
        let db = root.join("runs.sqlite");
        let persisted = run_store::persist_report(&db, &report).expect("persist prompt run");
        assert_eq!(persisted.run_records, 1);
        let list = run_store::list_runs(&db, run_store::RunListFilter::default())
            .expect("list persisted runs");
        (db, list.runs[0].run_id.clone())
    }

    #[test]
    fn publish_emits_the_declared_schema_and_field_set() {
        let root = temp_root("golden");
        let (db, run_id) = seed_run(&root, "OPENROUTER_API_KEY");
        let out = root.join("packets");

        let packet_path = publish(&run_id, &db, &out).expect("publish a real prompt_benchmark run");
        assert!(packet_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("publish-fixture-v0--test-publish-model--"));

        let bytes = std::fs::read(&packet_path).expect("read emitted packet");
        let packet: Value = serde_json::from_slice(&bytes).expect("packet is JSON");

        // Full key-set assertion (not a subset match) so schema drift on
        // either the top level or a nested object fails this test.
        let top_level: BTreeMap<String, Value> = packet
            .as_object()
            .expect("packet is a JSON object")
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let top_level_keys: Vec<&str> = top_level.keys().map(String::as_str).collect();
        assert_eq!(
            top_level_keys,
            vec![
                "benchmark",
                "classes",
                "config",
                "decision",
                "executed_at_unix_ms",
                "provenance",
                "response_model",
                "run_id",
                "schema_version",
                "score",
                "tasks",
                "title",
                "totals",
                "trusted",
            ]
        );

        assert_eq!(packet["schema_version"], BENCH_PACKET_SCHEMA);
        assert_eq!(packet["benchmark"], BENCHMARK);
        assert_eq!(packet["title"], "Publish Fixture Benchmark");
        assert_eq!(
            packet["decision"],
            "Ship the packet if the pass rate holds."
        );
        assert_eq!(packet["run_id"], run_id);
        assert_eq!(packet["trusted"], true);
        // Both tasks recorded the same `response_model` as `MODEL`, so the
        // aggregate is that uniform value, not the "disagreed/absent" null
        // sentinel `StoredRun::response_model` otherwise uses.
        assert_eq!(packet["response_model"], MODEL);

        let mut config_keys: Vec<&str> = packet["config"]
            .as_object()
            .expect("config is an object")
            .keys()
            .map(String::as_str)
            .collect();
        config_keys.sort_unstable();
        assert_eq!(
            config_keys,
            vec![
                "config_id",
                "max_tokens",
                "model",
                "provider",
                "system_prompt",
                "temperature",
            ]
        );
        assert_eq!(packet["config"]["provider"], "open_router");
        assert_eq!(packet["config"]["model"], MODEL);
        assert_eq!(packet["config"]["temperature"], 0);
        assert_eq!(packet["config"]["max_tokens"], 8);
        assert_eq!(packet["config"]["system_prompt"], "Answer exactly.");

        assert_eq!(packet["score"]["successes"], 1);
        assert_eq!(packet["score"]["n"], 2);
        assert_eq!(packet["score"]["method"], "Wilson");

        assert_eq!(
            packet["classes"],
            serde_json::json!([{ "class": "format_adherence", "successes": 1, "n": 2 }])
        );
        assert_eq!(packet["totals"]["tokens"], 14);
        assert_eq!(packet["totals"]["duration_ms"], 21);
        assert!((packet["totals"]["cost_usd"].as_f64().unwrap() - 0.002).abs() < 1e-9);

        let tasks = packet["tasks"].as_array().expect("tasks is an array");
        assert_eq!(tasks.len(), 2);
        let mut task_keys: Vec<&str> = tasks[0]
            .as_object()
            .expect("task is an object")
            .keys()
            .map(String::as_str)
            .collect();
        task_keys.sort_unstable();
        assert_eq!(
            task_keys,
            vec![
                "class",
                "cost_usd",
                "duration_ms",
                "expectation",
                "output",
                "passed",
                "prompt",
                "summary",
                "task_id",
                "tokens",
            ]
        );
        assert_eq!(tasks[0]["task_id"], "t0");
        assert_eq!(tasks[0]["prompt"], "What is the key?");
        assert_eq!(tasks[0]["summary"], "recall the exact key");
        assert_eq!(
            tasks[0]["expectation"],
            serde_json::json!({ "kind": "exact", "value": "match" })
        );
        assert_eq!(tasks[0]["output"], "match");
        assert_eq!(tasks[0]["passed"], true);
        assert_eq!(tasks[0]["tokens"], 7);
    }

    #[test]
    fn publish_refuses_an_unknown_run_id() {
        let root = temp_root("unknown-run");
        let (db, _run_id) = seed_run(&root, "OPENROUTER_API_KEY");
        let out = root.join("packets");

        let err = publish("does-not-exist", &db, &out).expect_err("unknown run id must refuse");
        assert!(
            format!("{err:#}").contains("not found"),
            "unexpected error: {err:#}"
        );
        assert!(!out.exists(), "no packet should be written on refusal");
    }

    #[test]
    fn publish_refuses_when_the_evidence_file_is_deleted() {
        let root = temp_root("deleted-evidence");
        let (db, run_id) = seed_run(&root, "OPENROUTER_API_KEY");
        let out = root.join("packets");

        let detail = run_store::show_run(&db, &run_id).expect("show seeded run");
        let evidence_path = detail
            .run
            .evidence_path
            .as_deref()
            .expect("seeded run recorded an evidence path");
        std::fs::remove_file(evidence_path).expect("delete evidence file");

        let err = publish(&run_id, &db, &out).expect_err("deleted evidence must refuse");
        assert!(
            format!("{err:#}").contains("could not read evidence file"),
            "unexpected error: {err:#}"
        );
        assert!(!out.exists(), "no packet should be written on refusal");
    }

    #[test]
    fn publish_refuses_on_an_evidence_spec_task_id_mismatch() {
        let root = temp_root("task-mismatch");
        let (db, run_id) = seed_run(&root, "OPENROUTER_API_KEY");
        let out = root.join("packets");

        let detail = run_store::show_run(&db, &run_id).expect("show seeded run");
        let evidence_path = detail
            .run
            .evidence_path
            .clone()
            .expect("seeded run recorded an evidence path");
        let mut evidence: Value =
            serde_json::from_str(&std::fs::read_to_string(&evidence_path).unwrap()).unwrap();
        evidence["tasks"][0]["task_id"] = serde_json::json!("task-not-in-spec");
        std::fs::write(
            &evidence_path,
            serde_json::to_string_pretty(&evidence).unwrap(),
        )
        .expect("rewrite evidence with a mismatched task id");

        let err =
            publish(&run_id, &db, &out).expect_err("an evidence/spec task-id mismatch must refuse");
        let message = format!("{err:#}");
        assert!(
            message.contains("task-not-in-spec"),
            "error should name the mismatched id: {message}"
        );
        assert!(!out.exists(), "no packet should be written on refusal");
    }

    #[test]
    fn publish_refuses_a_runner_kind_other_than_prompt_benchmark() {
        let root = temp_root("wrong-runner-kind");
        let out_dir = root.join("out");
        std::fs::create_dir_all(&out_dir).expect("create output dir");
        let spec_path = root.join("key-recall-fixture.json");
        std::fs::write(
            &spec_path,
            r#"{"schema_version":"crucible.eval_spec.v1","task":"key-recall-fixture"}"#,
        )
        .expect("write spec artifact");
        let report = RunReport {
            schema_version: RUN_REPORT_SCHEMA,
            output_dir: out_dir.display().to_string(),
            evals: vec![EvalReport {
                id: "key-recall-fixture".to_string(),
                title: "Key recall fixture".to_string(),
                score: Score {
                    metric: "pr_review_key_recall",
                    successes: 1,
                    n: 1,
                    point: Some(1.0),
                    lower: 0.0,
                    upper: 1.0,
                    confidence: 0.95,
                    method: "Wilson",
                },
                artifacts: vec![spec_path.display().to_string()],
                notes: Vec::new(),
            }],
        };
        let db = root.join("runs.sqlite");
        run_store::persist_report(&db, &report).expect("persist a built-in receipt run");
        let list = run_store::list_runs(&db, run_store::RunListFilter::default())
            .expect("list persisted runs");
        let run_id = list.runs[0].run_id.clone();

        let out = root.join("packets");
        let err = publish(&run_id, &db, &out).expect_err("a non-prompt_benchmark run must refuse");
        assert!(
            format!("{err:#}").contains("not yet publishable"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn publish_never_leaks_a_credential_value_present_in_the_process_environment() {
        // The honest guarantee: `publish` never reads process env at all, so
        // a fake credential VALUE set on the credential VAR NAME this spec
        // declares can never reach the packet — only the var *name* (e.g.
        // "OPENROUTER_API_KEY") is acceptable, and even that is omitted here
        // by not carrying `credential_env` into the packet shape at all.
        let root = temp_root("leak-check");
        // Assembled at runtime so the tracked source never contains a
        // credential-shaped literal (the repo leak-scan greps for shapes;
        // the runtime value still exercises the guarantee).
        let fake_token: String = ["sk", "test-fake-credential-do-not-use-6f3a9c21"].join("-");
        let fake_token: &str = &fake_token;
        // SAFETY: this test does not run concurrently with other tests that
        // read/write this exact env var name.
        unsafe {
            std::env::set_var("CRUCIBLE_PUBLISH_TEST_TOKEN", fake_token);
        }
        let (db, run_id) = seed_run(&root, "CRUCIBLE_PUBLISH_TEST_TOKEN");
        let out = root.join("packets");

        let packet_path = publish(&run_id, &db, &out).expect("publish with a credential_env set");
        let bytes = std::fs::read(&packet_path).expect("read emitted packet");
        let text = String::from_utf8(bytes).expect("packet is utf8");
        assert!(
            !text.contains(fake_token),
            "packet must never contain a live credential value"
        );

        // SAFETY: cleans up the env var this test set above.
        unsafe {
            std::env::remove_var("CRUCIBLE_PUBLISH_TEST_TOKEN");
        }
    }

    #[test]
    fn publish_tolerates_a_deleted_spec_file_with_null_spec_derived_fields() {
        let root = temp_root("deleted-spec");
        let (db, run_id) = seed_run(&root, "OPENROUTER_API_KEY");
        let detail = run_store::show_run(&db, &run_id).expect("show seeded run");
        let spec_path = detail
            .run
            .spec_path
            .clone()
            .expect("seeded run recorded a spec path");
        std::fs::remove_file(&spec_path).expect("delete spec file");
        let out = root.join("packets");

        let packet_path =
            publish(&run_id, &db, &out).expect("a deleted spec file is tolerated, not refused");
        let bytes = std::fs::read(&packet_path).expect("read emitted packet");
        let packet: Value = serde_json::from_slice(&bytes).expect("packet is JSON");
        assert_eq!(packet["title"], Value::Null);
        assert_eq!(packet["decision"], Value::Null);
        assert_eq!(packet["config"]["system_prompt"], Value::Null);
        assert_eq!(packet["tasks"][0]["prompt"], Value::Null);
        assert_eq!(packet["tasks"][0]["summary"], Value::Null);
        assert_eq!(packet["tasks"][0]["expectation"], Value::Null);
        // Evidence-owned facts are still present without the spec.
        assert_eq!(packet["tasks"][0]["output"], "match");
        assert_eq!(packet["tasks"][0]["passed"], true);
    }
}
