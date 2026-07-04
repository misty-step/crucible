//! Local HTTP application face for Crucible.
//!
//! This is intentionally the same shape as the adjudication server: a small
//! localhost-only stdlib HTTP loop over the existing Rust core. The browser UI
//! is static HTML/CSS/JS; the data comes from `validate`, `runs list`,
//! `runs show`, and the declared spec runner.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use crucible_core::{CorpusSpec, EvalSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{adjudication_panel, load_queue, run_store, spec_run, validate};

const SPECS_SCHEMA: &str = "crucible.ui.specs.v1";
const RUNS_SCHEMA: &str = "crucible.ui.runs.v1";
const ADJUDICATION_SCHEMA: &str = "crucible.ui.adjudication.v1";
const RUN_ACTION_SCHEMA: &str = "crucible.ui.run_action.v1";
const RUN_COMPARISON_SCHEMA: &str = "crucible.ui.run_comparison.v1";
const AESTHETIC_CSS: &str = include_str!("ui/aesthetic.css");

pub struct ServeOptions {
    pub db_path: PathBuf,
    pub specs_dir: PathBuf,
    pub port: u16,
}

pub fn serve(opts: ServeOptions) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", opts.port))
        .with_context(|| format!("binding 127.0.0.1:{}", opts.port))?;
    let bound_port = listener
        .local_addr()
        .map(|addr| addr.port())
        .unwrap_or(opts.port);
    println!("crucible serve: http://127.0.0.1:{bound_port}");
    std::io::stdout().flush().ok();

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("crucible serve: accept error: {err:#}");
                continue;
            }
        };
        if let Err(err) = handle_connection(stream, &opts) {
            eprintln!("crucible serve: connection error: {err:#}");
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream, opts: &ServeOptions) -> Result<()> {
    let request = HttpRequest::read(&stream)?;
    match route(&request, opts) {
        Ok(response) => response.write(&mut stream),
        Err(err) => {
            let body = json!({ "error": err.to_string() });
            HttpResponse::json(500, &body).write(&mut stream)
        }
    }
}

fn route(request: &HttpRequest, opts: &ServeOptions) -> Result<HttpResponse> {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => Ok(HttpResponse::html(render_index())),
        ("GET", "/favicon.ico") => Ok(HttpResponse::new(204, "image/x-icon", Vec::new())),
        ("GET", "/assets/aesthetic.css") => Ok(HttpResponse::new(
            200,
            "text/css; charset=utf-8",
            AESTHETIC_CSS.as_bytes().to_vec(),
        )),
        ("GET", "/api/specs") => HttpResponse::json_ok(&specs_response(&opts.specs_dir)?),
        ("GET", "/api/runs") => {
            HttpResponse::json_ok(&runs_response(&opts.db_path, &request.query)?)
        }
        ("GET", "/api/adjudication") => {
            HttpResponse::json_ok(&adjudication_response(&opts.db_path)?)
        }
        ("POST", "/api/run") => HttpResponse::json_ok(&run_spec_response(
            &opts.db_path,
            &opts.specs_dir,
            &request.body,
        )?),
        ("GET", path) if path.starts_with("/api/runs/") => {
            let raw = path.trim_start_matches("/api/runs/");
            let run_id = percent_decode(raw)?;
            HttpResponse::json_ok(&run_detail_response(&opts.db_path, &run_id)?)
        }
        ("GET", path) if path.starts_with("/adjudication/panel/") => {
            serve_adjudication_panel(path, &opts.db_path)
        }
        ("GET", path) if path.starts_with("/artifacts/") => serve_artifact(path, &opts.db_path),
        _ => Ok(HttpResponse::text(404, "not found")),
    }
}

struct HttpRequest {
    method: String,
    path: String,
    query: HashMap<String, String>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn read(stream: &TcpStream) -> Result<Self> {
        let mut reader = BufReader::new(stream.try_clone().context("cloning stream")?);
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .context("reading request line")?;
        if request_line.is_empty() {
            anyhow::bail!("empty request");
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let target = parts.next().unwrap_or("/").to_string();

        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).context("reading header")?;
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some((name, value)) = trimmed.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
        }

        let mut body = vec![0u8; content_length];
        if content_length > 0 {
            reader
                .read_exact(&mut body)
                .context("reading request body")?;
        }

        let (path, query) = split_target(&target)?;
        Ok(Self {
            method,
            path,
            query,
            body,
        })
    }
}

struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl HttpResponse {
    fn new(status: u16, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type,
            body,
        }
    }

    fn html(body: String) -> Self {
        Self::new(200, "text/html; charset=utf-8", body.into_bytes())
    }

    fn text(status: u16, body: &str) -> Self {
        Self::new(
            status,
            "text/plain; charset=utf-8",
            body.as_bytes().to_vec(),
        )
    }

    fn json<T: Serialize>(status: u16, value: &T) -> Self {
        let body =
            serde_json::to_vec_pretty(value).unwrap_or_else(|_| b"{\"error\":\"json\"}".to_vec());
        Self::new(status, "application/json", body)
    }

    fn json_ok<T: Serialize>(value: &T) -> Result<Self> {
        let body = serde_json::to_vec_pretty(value).context("serializing JSON response")?;
        Ok(Self::new(200, "application/json", body))
    }

    fn write(self, stream: &mut TcpStream) -> Result<()> {
        let status_text = match self.status {
            200 => "OK",
            204 => "No Content",
            400 => "Bad Request",
            404 => "Not Found",
            405 => "Method Not Allowed",
            500 => "Internal Server Error",
            _ => "OK",
        };
        write!(
            stream,
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.status,
            status_text,
            self.content_type,
            self.body.len()
        )
        .context("writing response headers")?;
        stream
            .write_all(&self.body)
            .context("writing response body")
    }
}

fn run_detail_response(db_path: &Path, run_id: &str) -> Result<Value> {
    let detail = run_store::show_run(db_path, run_id)?;
    let task_results = indexed_task_results(&detail)?;
    let mut value = serde_json::to_value(&detail).context("serializing run detail")?;
    value["task_results"] = task_results;
    value["adjudication_status"] = json!(adjudication_status(&detail));
    Ok(value)
}

fn indexed_task_results(detail: &run_store::RunDetail) -> Result<Value> {
    for artifact in &detail.artifacts {
        if artifact.path.ends_with("task-results.json") {
            let bytes = std::fs::read(&artifact.path)
                .with_context(|| format!("reading task results {}", artifact.path))?;
            let value: Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing task results {}", artifact.path))?;
            return Ok(value.get("tasks").cloned().unwrap_or_else(|| json!([])));
        }
    }
    Ok(json!([]))
}

fn adjudication_status(detail: &run_store::RunDetail) -> &'static str {
    if detail
        .artifacts
        .iter()
        .any(|artifact| artifact.path.ends_with("labels.json"))
    {
        "labels_present"
    } else if detail
        .artifacts
        .iter()
        .any(|artifact| artifact.path.ends_with("queue.json"))
    {
        "queue_present"
    } else {
        "not_indexed"
    }
}

fn split_target(target: &str) -> Result<(String, HashMap<String, String>)> {
    let (path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    let mut query = HashMap::new();
    for pair in raw_query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(percent_decode_query(key)?, percent_decode_query(value)?);
    }
    Ok((path.to_string(), query))
}

#[derive(Debug, Serialize)]
struct SpecsResponse {
    schema_version: &'static str,
    specs_dir: String,
    specs: Vec<SpecSummary>,
    load_errors: Vec<SpecLoadError>,
}

#[derive(Debug, Serialize)]
struct SpecSummary {
    path: String,
    id: String,
    object_label: &'static str,
    benchmark_title: String,
    plain_summary: String,
    task_count: Option<usize>,
    task_count_label: String,
    task_ids: Vec<String>,
    verifier_summary: String,
    runner_summary: String,
    supports_controlled_comparison: bool,
    runner_defaults: Option<RunnerDefaults>,
    task: String,
    inputs: String,
    outputs: String,
    decision: String,
    graders: Vec<GraderSummary>,
    baselines: Vec<String>,
    aggregation: String,
    uncertainty_method: String,
    confidence: f64,
    runner_kind: Option<String>,
    corpus: String,
    validation: validate::ValidationReport,
}

#[derive(Debug, Serialize)]
struct RunnerDefaults {
    provider: String,
    model: String,
    system_prompt: String,
    temperature: Option<u32>,
    max_output_units: Option<u32>,
    tool_policy: String,
    credential_env: String,
}

#[derive(Debug, Serialize)]
struct GraderSummary {
    id: String,
    kind: String,
}

#[derive(Debug, Serialize)]
struct SpecLoadError {
    path: String,
    error: String,
}

fn specs_response(specs_dir: &Path) -> Result<SpecsResponse> {
    let mut paths = json_files(specs_dir)?;
    paths.sort();
    let mut specs = Vec::new();
    let mut load_errors = Vec::new();
    for path in paths {
        match (validate::validate(&path), spec_run::load_spec(&path)) {
            (Ok(validation), Ok(spec)) => specs.push(spec_summary(path, spec, validation)),
            (Err(err), _) | (_, Err(err)) => load_errors.push(SpecLoadError {
                path: display_path(&path),
                error: err.to_string(),
            }),
        }
    }
    for error in &load_errors {
        specs.push(load_error_spec_summary(error));
    }
    Ok(SpecsResponse {
        schema_version: SPECS_SCHEMA,
        specs_dir: display_path(specs_dir),
        specs,
        load_errors,
    })
}

fn load_error_spec_summary(error: &SpecLoadError) -> SpecSummary {
    let path = Path::new(&error.path);
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(&error.path)
        .to_string();
    SpecSummary {
        path: error.path.clone(),
        id,
        object_label: "benchmark",
        benchmark_title: "Unloaded benchmark".to_string(),
        plain_summary:
            "This benchmark file could not be parsed, so Crucible cannot explain or run it yet."
                .to_string(),
        task_count: None,
        task_count_label: "unknown task count".to_string(),
        task_ids: Vec::new(),
        verifier_summary: "No verifier available until the file loads.".to_string(),
        runner_summary: "Not runnable.".to_string(),
        supports_controlled_comparison: false,
        runner_defaults: None,
        task: "load-error".to_string(),
        inputs: String::new(),
        outputs: String::new(),
        decision: String::new(),
        graders: Vec::new(),
        baselines: Vec::new(),
        aggregation: "unknown".to_string(),
        uncertainty_method: "unknown".to_string(),
        confidence: 0.95,
        runner_kind: None,
        corpus: "unloaded".to_string(),
        validation: validate::ValidationReport {
            schema_version: validate::VALIDATE_REPORT_SCHEMA,
            spec: error.path.clone(),
            valid: false,
            runnable: false,
            errors: vec![validate::ValidationIssue {
                field: "load".to_string(),
                message: error.error.clone(),
            }],
            warnings: Vec::new(),
        },
    }
}

fn spec_summary(
    path: PathBuf,
    spec: EvalSpec,
    validation: validate::ValidationReport,
) -> SpecSummary {
    let runner_kind = spec.runner.as_ref().map(|runner| json_string(&runner.kind));
    let runner_defaults = spec
        .runner
        .as_ref()
        .and_then(|runner| runner_defaults(&runner.corpus));
    let task_count = spec
        .runner
        .as_ref()
        .and_then(|runner| corpus_task_count(&runner.corpus));
    let task_count_label = task_count
        .map(|count| format!("{count} task{}", plural(count)))
        .unwrap_or_else(|| "task count depends on the selected corpus".to_string());
    let task_ids = spec
        .runner
        .as_ref()
        .map(|runner| corpus_task_ids(&runner.corpus))
        .unwrap_or_default();
    let verifier_summary = verifier_summary(&spec);
    let runner_summary = spec
        .runner
        .as_ref()
        .map(|runner| runner_plain_summary(&runner.corpus))
        .unwrap_or_else(|| "Definition-only benchmark; no runner is declared.".to_string());
    let supports_controlled_comparison = supports_controlled_comparison(&spec);
    let benchmark_title = if spec.task.is_empty() {
        spec.id.clone()
    } else {
        spec.task.clone()
    };
    let plain_summary = plain_benchmark_summary(&spec, task_count);
    let corpus = spec
        .runner
        .as_ref()
        .map(|runner| corpus_summary(&runner.corpus))
        .unwrap_or_else(|| "definition_only".to_string());
    SpecSummary {
        path: display_path(&path),
        id: spec.id,
        object_label: "benchmark",
        benchmark_title,
        plain_summary,
        task_count,
        task_count_label,
        task_ids,
        verifier_summary,
        runner_summary,
        supports_controlled_comparison,
        runner_defaults,
        task: spec.task,
        inputs: spec.inputs,
        outputs: spec.outputs,
        decision: spec.decision,
        graders: spec
            .graders
            .graders
            .into_iter()
            .map(|grader| GraderSummary {
                id: grader.id,
                kind: json_string(&grader.kind),
            })
            .collect(),
        baselines: spec.baselines,
        aggregation: json_string(&spec.aggregation),
        uncertainty_method: json_string(&spec.uncertainty.method),
        confidence: spec.uncertainty.confidence,
        runner_kind,
        corpus,
        validation,
    }
}

fn supports_controlled_comparison(spec: &EvalSpec) -> bool {
    spec.runner
        .as_ref()
        .map(|runner| {
            json_string(&runner.kind) == "prompt_benchmark"
                && spec
                    .graders
                    .graders
                    .iter()
                    .any(|grader| json_string(&grader.kind) == "deterministic")
        })
        .unwrap_or(false)
}

fn plain_benchmark_summary(spec: &EvalSpec, task_count: Option<usize>) -> String {
    let count = task_count
        .map(|count| format!("{count} task{}", plural(count)))
        .unwrap_or_else(|| "a declared task corpus".to_string());
    if !spec.inputs.is_empty() {
        format!("Tests {} across {count}.", sentence_fragment(&spec.inputs))
    } else if !spec.decision.is_empty() {
        spec.decision.clone()
    } else {
        format!("Tests {} across {count}.", spec.task)
    }
}

fn verifier_summary(spec: &EvalSpec) -> String {
    let kinds: Vec<_> = spec
        .graders
        .graders
        .iter()
        .map(|grader| json_string(&grader.kind))
        .collect();
    if kinds.iter().any(|kind| kind == "agentic") {
        "Verifier-authoring artifact: this spec uses a judge model and is not a run-time deterministic benchmark.".to_string()
    } else if kinds.iter().any(|kind| kind == "human") {
        "Verifier-authoring artifact: this spec depends on human labels outside the run."
            .to_string()
    } else if kinds.iter().any(|kind| kind == "deterministic") {
        match spec.runner.as_ref().map(|runner| &runner.corpus) {
            Some(CorpusSpec::PromptBenchmark { tasks, .. }) => {
                let kinds = expectation_kinds(tasks);
                format!("Deterministic text verifier: {}.", kinds.join(", "))
            }
            Some(CorpusSpec::DaedalusTrials { .. })
            | Some(CorpusSpec::CerberusReceiptBundles { .. }) => {
                "Deterministic scorer key: matches produced findings against expected rows."
                    .to_string()
            }
            _ => "Deterministic verifier declared.".to_string(),
        }
    } else {
        "No verifier is declared yet.".to_string()
    }
}

fn runner_plain_summary(corpus: &CorpusSpec) -> String {
    match corpus {
        CorpusSpec::PromptBenchmark { config, tasks } => format!(
            "Runs {} prompt task{} through {:?}/{} with text-only model calls.",
            tasks.len(),
            plural(tasks.len()),
            config.provider,
            config.model
        ),
        CorpusSpec::DaedalusTrials {
            candidate_id,
            tasks,
            ..
        } => format!(
            "Reads saved Threshold trials for candidate {candidate_id}; selected tasks: {}.",
            if tasks.is_empty() {
                "all".to_string()
            } else {
                tasks.len().to_string()
            }
        ),
        CorpusSpec::CerberusReceiptBundles {
            candidate_id,
            tasks,
        } => format!(
            "Grades {} Cerberus receipt task{} for candidate {candidate_id}.",
            tasks.len(),
            plural(tasks.len())
        ),
        CorpusSpec::AgenticJudge { config, tasks } => format!(
            "Runs {} judge task{} through {:?}/{}; shown only as verifier-authoring evidence.",
            tasks.len(),
            plural(tasks.len()),
            config.provider,
            config.model
        ),
    }
}

fn runner_defaults(corpus: &CorpusSpec) -> Option<RunnerDefaults> {
    let CorpusSpec::PromptBenchmark { config, .. } = corpus else {
        return None;
    };
    Some(RunnerDefaults {
        provider: json_string(&config.provider),
        model: config.model.clone(),
        system_prompt: config.system_prompt.clone(),
        temperature: config.temperature,
        max_output_units: config.max_output_units,
        tool_policy: "No tools. The runner sends one text prompt to the model and grades the final text with deterministic verifiers.".to_string(),
        credential_env: config.credential_env.clone(),
    })
}

fn corpus_task_count(corpus: &CorpusSpec) -> Option<usize> {
    match corpus {
        CorpusSpec::PromptBenchmark { tasks, .. } => Some(tasks.len()),
        CorpusSpec::AgenticJudge { tasks, .. } => Some(tasks.len()),
        CorpusSpec::CerberusReceiptBundles { tasks, .. } => Some(tasks.len()),
        CorpusSpec::DaedalusTrials { tasks, .. } if !tasks.is_empty() => Some(tasks.len()),
        CorpusSpec::DaedalusTrials { .. } => None,
    }
}

fn corpus_task_ids(corpus: &CorpusSpec) -> Vec<String> {
    match corpus {
        CorpusSpec::PromptBenchmark { tasks, .. } => {
            tasks.iter().map(|task| task.task_id.clone()).collect()
        }
        CorpusSpec::AgenticJudge { tasks, .. } => {
            tasks.iter().map(|task| task.task_id.clone()).collect()
        }
        CorpusSpec::CerberusReceiptBundles { tasks, .. } => {
            tasks.iter().map(|task| task.task_id.clone()).collect()
        }
        CorpusSpec::DaedalusTrials { tasks, .. } => tasks.clone(),
    }
}

fn expectation_kinds(tasks: &[crucible_core::PromptBenchmarkTask]) -> Vec<String> {
    let mut kinds = Vec::new();
    for task in tasks {
        let kind = match &task.expectation {
            crucible_core::PromptExpectation::Exact { .. } => "exact match",
            crucible_core::PromptExpectation::Contains { .. } => "contains check",
            crucible_core::PromptExpectation::CaseInsensitiveContains { .. } => {
                "case-insensitive contains check"
            }
            crucible_core::PromptExpectation::Regex { .. } => "regex match",
            crucible_core::PromptExpectation::StrictJson { .. } => "strict JSON",
            crucible_core::PromptExpectation::PythonUnitTest { .. } => "Python unit test",
        };
        if !kinds.iter().any(|existing| existing == kind) {
            kinds.push(kind.to_string());
        }
    }
    if kinds.is_empty() {
        kinds.push("no tasks".to_string());
    }
    kinds
}

fn sentence_fragment(text: &str) -> String {
    let trimmed = text.trim().trim_end_matches('.');
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => "the declared behavior".to_string(),
    }
}

fn corpus_summary(corpus: &CorpusSpec) -> String {
    match corpus {
        CorpusSpec::DaedalusTrials {
            candidate_id,
            tasks,
            ..
        } => format!(
            "daedalus_trials candidate={} tasks={}",
            candidate_id,
            if tasks.is_empty() {
                "all".to_string()
            } else {
                tasks.len().to_string()
            }
        ),
        CorpusSpec::CerberusReceiptBundles {
            candidate_id,
            tasks,
            ..
        } => format!(
            "cerberus_receipt_bundles candidate={} tasks={}",
            candidate_id,
            tasks.len()
        ),
        CorpusSpec::PromptBenchmark { config, tasks } => {
            format!(
                "prompt_benchmark model={} tasks={}",
                config.model,
                tasks.len()
            )
        }
        CorpusSpec::AgenticJudge { config, tasks } => {
            format!("agentic_judge model={} tasks={}", config.model, tasks.len())
        }
    }
}

fn json_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry.context("reading directory entry")?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            out.push(path);
        }
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
struct RunsResponse {
    schema_version: &'static str,
    db: String,
    filters: RunFilters,
    runs: Vec<run_store::StoredRun>,
    trendlines: Vec<Trendline>,
}

#[derive(Debug, Default, Serialize)]
struct RunFilters {
    benchmark: Option<String>,
    config: Option<String>,
    model: Option<String>,
    since: Option<String>,
    until: Option<String>,
}

#[derive(Debug, Serialize)]
struct Trendline {
    benchmark_id: String,
    points: Vec<TrendPoint>,
}

#[derive(Debug, Serialize)]
struct TrendPoint {
    run_id: String,
    created_at_unix_ms: i64,
    config_id: String,
    model: Option<String>,
    point: Option<f64>,
    lower: f64,
    upper: f64,
}

fn runs_response(db_path: &Path, query: &HashMap<String, String>) -> Result<RunsResponse> {
    let filters = RunFilters {
        benchmark: nonempty_query(query, "benchmark"),
        config: nonempty_query(query, "config"),
        model: nonempty_query(query, "model"),
        since: nonempty_query(query, "since"),
        until: nonempty_query(query, "until"),
    };
    let since_unix_ms = filters
        .since
        .as_deref()
        .map(run_store::parse_timestamp_bound)
        .transpose()?;
    let until_unix_ms = filters
        .until
        .as_deref()
        .map(run_store::parse_timestamp_bound)
        .transpose()?;
    let list = run_store::list_runs(
        db_path,
        run_store::RunListFilter {
            benchmark: filters.benchmark.as_deref(),
            config: filters.config.as_deref(),
            model: filters.model.as_deref(),
            since_unix_ms,
            until_unix_ms,
        },
    )?;
    let trendlines = trendlines(&list.runs);
    Ok(RunsResponse {
        schema_version: RUNS_SCHEMA,
        db: list.db,
        filters,
        runs: list.runs,
        trendlines,
    })
}

fn trendlines(runs: &[run_store::StoredRun]) -> Vec<Trendline> {
    let mut by_benchmark: BTreeMap<String, Vec<TrendPoint>> = BTreeMap::new();
    for run in runs.iter().rev() {
        by_benchmark
            .entry(run.benchmark_id.clone())
            .or_default()
            .push(TrendPoint {
                run_id: run.run_id.clone(),
                created_at_unix_ms: run.created_at_unix_ms,
                config_id: run.config_id.clone(),
                model: run.model.clone(),
                point: run.point,
                lower: run.lower,
                upper: run.upper,
            });
    }
    by_benchmark
        .into_iter()
        .map(|(benchmark_id, points)| Trendline {
            benchmark_id,
            points,
        })
        .collect()
}

#[derive(Debug, Serialize)]
struct AdjudicationResponse {
    schema_version: &'static str,
    panels: Vec<AdjudicationPanelLink>,
}

#[derive(Debug, Serialize)]
struct AdjudicationPanelLink {
    run_id: String,
    benchmark_id: String,
    title: String,
    queue_path: Option<String>,
    queue_url: Option<String>,
    panel_path: Option<String>,
    panel_url: Option<String>,
}

fn adjudication_response(db_path: &Path) -> Result<AdjudicationResponse> {
    let list = run_store::list_runs(db_path, run_store::RunListFilter::default())?;
    let mut panels = Vec::new();
    for run in list.runs {
        let detail = run_store::show_run(db_path, &run.run_id)?;
        let queue = detail.artifacts.iter().enumerate().find(|(_, artifact)| {
            artifact.path.ends_with("queue.json") && !artifact.path.contains("/panel/")
        });
        let queue = queue.or_else(|| {
            detail
                .artifacts
                .iter()
                .enumerate()
                .find(|(_, artifact)| artifact.path.ends_with("queue.json"))
        });
        let panel = detail
            .artifacts
            .iter()
            .enumerate()
            .find(|(_, artifact)| artifact.path.ends_with("panel/index.html"));
        if queue.is_none() && panel.is_none() {
            continue;
        }
        panels.push(AdjudicationPanelLink {
            run_id: run.run_id.clone(),
            benchmark_id: run.benchmark_id,
            title: run.title,
            queue_path: queue.map(|(_, artifact)| artifact.path.clone()),
            queue_url: queue.map(|(index, _)| artifact_url(&run.run_id, index)),
            panel_path: panel.map(|(_, artifact)| artifact.path.clone()),
            panel_url: queue
                .map(|_| adjudication_panel_url(&run.run_id))
                .or_else(|| panel.map(|(index, _)| artifact_url(&run.run_id, index))),
        });
    }
    Ok(AdjudicationResponse {
        schema_version: ADJUDICATION_SCHEMA,
        panels,
    })
}

#[derive(Debug, Deserialize)]
struct RunSpecRequest {
    spec: String,
    out: Option<String>,
    runners: Option<Vec<RunnerRequest>>,
    alpha: Option<f64>,
}

#[derive(Debug, Serialize)]
struct RunSpecResponse {
    schema_version: &'static str,
    spec: String,
    output_dir: String,
    mode: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stored: Option<run_store::PersistedReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    report: Option<crate::eval_run::RunReport>,
    runs: Vec<RunActionRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comparison: Option<RunComparisonResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comparison_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RunnerRequest {
    id: Option<String>,
    model: String,
    system_prompt: Option<String>,
    temperature: Option<u32>,
    max_output_units: Option<u32>,
}

#[derive(Debug, Serialize)]
struct RunActionRun {
    runner_id: String,
    model: String,
    output_dir: String,
    invocation_id: String,
    run_id: Option<String>,
    config_id: Option<String>,
    benchmark_id: Option<String>,
    point: Option<f64>,
    lower: Option<f64>,
    upper: Option<f64>,
    successes: Option<u64>,
    n: Option<u64>,
    report: crate::eval_run::RunReport,
    stored: run_store::PersistedReport,
}

#[derive(Debug, Serialize)]
struct RunComparisonResponse {
    schema_version: &'static str,
    changed_variables: Vec<String>,
    control_label: String,
    verdict_explanation: String,
    comparison: run_store::ConfigComparison,
}

fn run_spec_response(db_path: &Path, specs_dir: &Path, body: &[u8]) -> Result<RunSpecResponse> {
    let request: RunSpecRequest =
        serde_json::from_slice(body).context("parsing run request JSON body")?;
    let spec_path = resolve_requested_spec(specs_dir, &request.spec)?;
    let runners = request.runners.unwrap_or_default();
    if runners.is_empty() {
        return run_single_spec(db_path, &spec_path, request.out);
    }
    run_controlled_comparison(
        db_path,
        &spec_path,
        request.out,
        runners,
        request.alpha.unwrap_or(run_store::DEFAULT_ALPHA),
    )
}

fn run_single_spec(
    db_path: &Path,
    spec_path: &Path,
    out: Option<String>,
) -> Result<RunSpecResponse> {
    let out_dir = out
        .map(PathBuf::from)
        .unwrap_or_else(|| default_run_out(spec_path));
    let report = spec_run::run(spec_path, Some(&out_dir))?;
    let stored = run_store::persist_report(db_path, &report)?;
    let run = stored_run_for_invocation(db_path, &stored.invocation_id)?;
    Ok(RunSpecResponse {
        schema_version: RUN_ACTION_SCHEMA,
        spec: display_path(spec_path),
        output_dir: report.output_dir.clone(),
        mode: "single",
        stored: Some(stored.clone()),
        report: Some(report.clone()),
        runs: vec![run_action_run(
            "default".to_string(),
            run.as_ref()
                .and_then(|run| run.model.clone())
                .unwrap_or_else(|| "deterministic".to_string()),
            report,
            stored,
            run,
        )],
        comparison: None,
        comparison_error: None,
    })
}

fn run_controlled_comparison(
    db_path: &Path,
    spec_path: &Path,
    out: Option<String>,
    runners: Vec<RunnerRequest>,
    alpha: f64,
) -> Result<RunSpecResponse> {
    if runners.len() < 2 {
        anyhow::bail!("a controlled comparison needs at least two runners");
    }
    let spec = spec_run::load_spec(spec_path)?;
    if !supports_controlled_comparison(&spec) {
        anyhow::bail!(
            "controlled comparison is currently available for deterministic prompt_benchmark specs"
        );
    }
    let base_out = out
        .map(PathBuf::from)
        .unwrap_or_else(|| default_run_out(spec_path));
    let mut run_rows = Vec::new();
    for (index, runner) in runners.into_iter().enumerate() {
        let runner_id = runner
            .id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("runner-{}", index + 1));
        let model = runner.model.trim().to_string();
        if model.is_empty() {
            anyhow::bail!("{runner_id} must declare a model");
        }
        let output_dir = base_out.join(format!(
            "{}-{}",
            safe_path_component(&runner_id),
            safe_path_component(&model)
        ));
        let options = spec_run::RunOptions {
            prompt_model: Some(model.clone()),
            prompt_system_prompt: runner.system_prompt,
            prompt_max_output_units: runner.max_output_units,
            prompt_temperature: runner.temperature,
        };
        let report = spec_run::run_with_options(spec_path, Some(&output_dir), &options)?;
        let stored = run_store::persist_report(db_path, &report)?;
        let row = stored_run_for_invocation(db_path, &stored.invocation_id)?;
        run_rows.push(run_action_run(runner_id, model, report, stored, row));
    }

    let changed_variables = changed_variables(&run_rows);
    let comparison = if run_rows.len() >= 2 {
        let left = run_rows[0]
            .config_id
            .as_deref()
            .unwrap_or(run_rows[0].model.as_str());
        let right = run_rows[1]
            .config_id
            .as_deref()
            .unwrap_or(run_rows[1].model.as_str());
        let benchmark = run_rows[0]
            .benchmark_id
            .as_deref()
            .or_else(|| {
                run_rows[0]
                    .report
                    .evals
                    .first()
                    .map(|eval| eval.id.as_str())
            })
            .with_context(|| "first runner produced no benchmark id")?;
        match run_store::compare_configs(db_path, benchmark, left, right, alpha) {
            Ok(comparison) => Some(RunComparisonResponse {
                schema_version: RUN_COMPARISON_SCHEMA,
                control_label: control_label(&changed_variables),
                verdict_explanation: verdict_explanation(&comparison),
                changed_variables,
                comparison,
            }),
            Err(err) => {
                return Ok(RunSpecResponse {
                    schema_version: RUN_ACTION_SCHEMA,
                    spec: display_path(spec_path),
                    output_dir: base_out.display().to_string(),
                    mode: "controlled_comparison",
                    stored: None,
                    report: None,
                    runs: run_rows,
                    comparison: None,
                    comparison_error: Some(err.to_string()),
                });
            }
        }
    } else {
        None
    };

    Ok(RunSpecResponse {
        schema_version: RUN_ACTION_SCHEMA,
        spec: display_path(spec_path),
        output_dir: base_out.display().to_string(),
        mode: "controlled_comparison",
        stored: None,
        report: None,
        runs: run_rows,
        comparison,
        comparison_error: None,
    })
}

fn run_action_run(
    runner_id: String,
    model: String,
    report: crate::eval_run::RunReport,
    stored: run_store::PersistedReport,
    row: Option<run_store::StoredRun>,
) -> RunActionRun {
    RunActionRun {
        runner_id,
        model,
        output_dir: report.output_dir.clone(),
        invocation_id: stored.invocation_id.clone(),
        run_id: row.as_ref().map(|row| row.run_id.clone()),
        config_id: row.as_ref().map(|row| row.config_id.clone()),
        benchmark_id: row.as_ref().map(|row| row.benchmark_id.clone()),
        point: row.as_ref().and_then(|row| row.point),
        lower: row.as_ref().map(|row| row.lower),
        upper: row.as_ref().map(|row| row.upper),
        successes: row.as_ref().map(|row| row.successes),
        n: row.as_ref().map(|row| row.n),
        report,
        stored,
    }
}

fn stored_run_for_invocation(
    db_path: &Path,
    invocation_id: &str,
) -> Result<Option<run_store::StoredRun>> {
    let list = run_store::list_runs(db_path, run_store::RunListFilter::default())?;
    Ok(list
        .runs
        .into_iter()
        .find(|run| run.invocation_id == invocation_id))
}

fn changed_variables(runs: &[RunActionRun]) -> Vec<String> {
    if runs.len() < 2 {
        return Vec::new();
    }
    let mut changed = Vec::new();
    if unique_count(runs.iter().map(|run| run.model.as_str())) > 1 {
        changed.push("model".to_string());
    }
    if unique_count(
        runs.iter()
            .filter_map(|run| run.config_id.as_deref())
            .map(config_identity_without_model),
    ) > 1
    {
        changed.push("prompt or parameters".to_string());
    }
    if changed.is_empty() {
        changed.push("none detected".to_string());
    }
    changed
}

fn unique_count<'a>(values: impl Iterator<Item = &'a str>) -> usize {
    let mut set = std::collections::BTreeSet::new();
    for value in values {
        set.insert(value.to_string());
    }
    set.len()
}

fn config_identity_without_model(config_id: &str) -> &str {
    config_id
        .split_once(":temp=")
        .map(|(_, rest)| rest)
        .unwrap_or(config_id)
}

fn control_label(changed_variables: &[String]) -> String {
    if changed_variables.len() == 1 && changed_variables[0] == "model" {
        "Controlled comparison: only the model changed.".to_string()
    } else if changed_variables.len() == 1 && changed_variables[0] == "none detected" {
        "Same runner configuration on both sides; this is a repeatability check.".to_string()
    } else {
        format!(
            "Multi-variable comparison: {} changed.",
            changed_variables.join(", ")
        )
    }
}

fn verdict_explanation(comparison: &run_store::ConfigComparison) -> String {
    if let Some(paired) = &comparison.paired {
        let interval = format!(
            "{} shared task{}",
            comparison.common_tasks,
            plural(comparison.common_tasks)
        );
        match paired.verdict {
            crucible_core::DeltaVerdict::Signal => {
                format!("The paired tasks clear the noise floor over {interval}; this is evidence of a real difference for this benchmark.")
            }
            crucible_core::DeltaVerdict::InsideNoiseFloor => {
                format!("The shared tasks do not clear the noise floor over {interval}; with this sample size, treat the result as inconclusive and run more tasks.")
            }
        }
    } else {
        "Crucible could not pair shared task rows, so this is only a latest-run score difference and not a significance claim.".to_string()
    }
}

fn resolve_requested_spec(specs_dir: &Path, requested: &str) -> Result<PathBuf> {
    let requested_path = PathBuf::from(requested);
    let requested_abs = lexical_normalize(&if requested_path.is_absolute() {
        requested_path
    } else {
        std::env::current_dir()
            .context("reading current directory")?
            .join(requested_path)
    });
    for path in json_files(specs_dir)? {
        let abs = lexical_normalize(&if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()
                .context("reading current directory")?
                .join(&path)
        });
        if abs == requested_abs {
            return Ok(path);
        }
    }
    anyhow::bail!(
        "{requested:?} is not a known spec under {}",
        specs_dir.display()
    )
}

fn default_run_out(spec_path: &Path) -> PathBuf {
    let id = spec_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("spec")
        .replace('/', "-");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    Path::new("runs")
        .join("local")
        .join(format!("ui-{id}-{now}"))
}

fn serve_artifact(path: &str, db_path: &Path) -> Result<HttpResponse> {
    let rest = path.trim_start_matches("/artifacts/");
    let Some((run_id_raw, index_raw)) = rest.rsplit_once('/') else {
        return Ok(HttpResponse::text(404, "not found"));
    };
    let run_id = percent_decode(run_id_raw)?;
    let index: usize = index_raw.parse().context("parsing artifact index")?;
    let detail = run_store::show_run(db_path, &run_id)?;
    let Some(artifact) = detail.artifacts.get(index) else {
        return Ok(HttpResponse::text(404, "not found"));
    };
    let bytes = std::fs::read(&artifact.path)
        .with_context(|| format!("reading artifact {}", artifact.path))?;
    let content_type = if artifact.path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if artifact.path.ends_with(".json") {
        "application/json"
    } else if artifact.path.ends_with(".md") {
        "text/markdown; charset=utf-8"
    } else {
        "application/octet-stream"
    };
    Ok(HttpResponse::new(200, content_type, bytes))
}

fn serve_adjudication_panel(path: &str, db_path: &Path) -> Result<HttpResponse> {
    let raw = path.trim_start_matches("/adjudication/panel/");
    let run_id = percent_decode(raw)?;
    let detail = run_store::show_run(db_path, &run_id)?;
    let queue_artifact = detail
        .artifacts
        .iter()
        .find(|artifact| {
            artifact.path.ends_with("queue.json") && !artifact.path.contains("/panel/")
        })
        .or_else(|| {
            detail
                .artifacts
                .iter()
                .find(|artifact| artifact.path.ends_with("queue.json"))
        });
    if let Some(queue_artifact) = queue_artifact {
        let queue = load_queue(Path::new(&queue_artifact.path))?;
        return Ok(HttpResponse::html(adjudication_panel::render(&queue)));
    }

    let panel_path = detail
        .artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with("panel/index.html"))
        .with_context(|| format!("run {run_id:?} has no adjudication panel artifact"))?
        .path
        .clone();
    let html = std::fs::read_to_string(&panel_path)
        .with_context(|| format!("reading panel artifact {panel_path}"))?;
    Ok(HttpResponse::html(html))
}

fn artifact_url(run_id: &str, index: usize) -> String {
    format!("/artifacts/{}/{}", percent_encode(run_id), index)
}

fn adjudication_panel_url(run_id: &str) -> String {
    format!("/adjudication/panel/{}", percent_encode(run_id))
}

fn nonempty_query(query: &HashMap<String, String>, key: &str) -> Option<String> {
    query.get(key).filter(|value| !value.is_empty()).cloned()
}

fn json_string<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn safe_path_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "runner".to_string()
    } else {
        trimmed.to_string()
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn percent_decode(raw: &str) -> Result<String> {
    percent_decode_inner(raw, false)
}

fn percent_decode_query(raw: &str) -> Result<String> {
    percent_decode_inner(raw, true)
}

fn percent_decode_inner(raw: &str, plus_as_space: bool) -> Result<String> {
    let mut bytes = Vec::with_capacity(raw.len());
    let raw = raw.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'%' if index + 2 < raw.len() => {
                let hex = std::str::from_utf8(&raw[index + 1..index + 3])
                    .context("decoding percent escape")?;
                let byte = u8::from_str_radix(hex, 16).context("parsing percent escape")?;
                bytes.push(byte);
                index += 3;
            }
            b'+' if plus_as_space => {
                bytes.push(b' ');
                index += 1;
            }
            byte => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(bytes).context("percent-decoded path is utf-8")
}

fn percent_encode(raw: &str) -> String {
    let mut out = String::new();
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn render_index() -> String {
    r#"<!doctype html>
<html lang="en" data-ae-mode="light">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
  <title>Crucible benchmark arena</title>
  <link rel="icon" type="image/svg+xml" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%231a1a1a' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2'/%3E%3Cpath d='M6.453 15h11.094'/%3E%3Cpath d='M8.5 2h7'/%3E%3C/svg%3E">
  <link rel="stylesheet" href="/assets/aesthetic.css">
  <style>
    :root { --ae-accent: #8a3b30; --ae-accent-dark: #ff9f90; }
    .cru-desk { display: grid; gap: var(--ae-space-5); align-content: start; }
    .cru-toolbar { display: flex; gap: var(--ae-space-3); align-items: start; justify-content: space-between; flex-wrap: wrap; }
    .cru-title { font-weight: var(--ae-w-black); }
    .cru-lede { color: var(--ae-ink-muted); max-width: 58rem; }
    .cru-subtle { color: var(--ae-ink-muted); }
    .cru-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--ae-space-4); }
    .cru-grid.two { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .cru-grid.four { grid-template-columns: repeat(4, minmax(0, 1fr)); }
    .cru-card { border: 1px solid var(--ae-line); background: var(--ae-surface); padding: var(--ae-space-5); display: grid; gap: var(--ae-space-3); align-content: start; }
    .cru-card.selected { border-color: var(--ae-ink); box-shadow: inset 0 0 0 1px var(--ae-ink); }
    .cru-card.warning { background: var(--ae-wash); }
    .cru-actions { display: flex; gap: var(--ae-space-2); align-items: center; flex-wrap: wrap; }
    .cru-button { appearance: none; border: 1px solid var(--ae-ink); background: var(--ae-ink); color: var(--ae-surface); padding: .55em .8em; border-radius: 0; cursor: pointer; transition: transform var(--ae-quick) var(--ae-ease), background var(--ae-quick) var(--ae-ease); }
    .cru-button:hover { transform: translateY(-1px); }
    .cru-button:active { transform: translateY(0); }
    .cru-button.secondary { background: transparent; color: var(--ae-ink); border-color: var(--ae-line); }
    .cru-button:disabled { opacity: .45; cursor: default; transform: none; }
    .cru-input, .cru-textarea, .cru-select { appearance: none; border: 1px solid var(--ae-line); background: var(--ae-surface); color: var(--ae-ink); padding: .55em .65em; border-radius: 0; min-width: 0; width: 100%; box-sizing: border-box; }
    .cru-textarea { min-height: 9em; resize: vertical; line-height: 1.45; }
    .cru-field { display: grid; gap: var(--ae-space-1); }
    .cru-label { font-family: var(--ae-font-mono); font-size: 13px; color: var(--ae-ink-muted); }
    .cru-chipline { display: flex; gap: .45em; flex-wrap: wrap; }
    .cru-chip { border: 1px solid var(--ae-line); padding: .18em .48em; font-family: var(--ae-font-mono); font-size: 13px; background: var(--ae-wash); }
    .cru-chip.ok { color: var(--ae-ok); }
    .cru-chip.warn { color: var(--ae-warn); }
    .cru-chip.err { color: var(--ae-err); }
    .cru-status { display: inline-flex; gap: .35em; align-items: baseline; }
    .cru-status.ok .glyph { color: var(--ae-ok); }
    .cru-status.warn .glyph { color: var(--ae-warn); }
    .cru-status.err .glyph { color: var(--ae-err); }
    .cru-code { font-family: var(--ae-font-mono); font-size: 13px; word-break: break-word; }
    .cru-empty { border: 1px solid var(--ae-line); padding: var(--ae-space-5); color: var(--ae-ink-muted); }
    .cru-table-wrap { overflow: auto; border: 1px solid var(--ae-line); }
    .cru-table-wrap .ae-table th, .cru-table-wrap .ae-table td { white-space: nowrap; vertical-align: top; }
    .cru-table-wrap .ae-table td.wrap { white-space: normal; min-width: 14em; }
    .cru-click { cursor: pointer; }
    .cru-click:hover td { background: var(--ae-wash); }
    .cru-ci { position: relative; height: 1.45em; border-bottom: 1px solid var(--ae-line); margin-top: .35em; min-width: 12em; }
    .cru-ci .band { position: absolute; top: .58em; height: 4px; background: var(--ae-wash); border: 1px solid var(--ae-line); }
    .cru-ci .point { position: absolute; top: .25em; width: 1px; height: 1em; background: var(--ae-ink); }
    .cru-progress { display: grid; grid-template-columns: minmax(12em, .7fr) repeat(2, minmax(10em, 1fr)); border: 1px solid var(--ae-line); }
    .cru-progress > div { padding: .65em .75em; border-left: 1px solid var(--ae-line); border-top: 1px solid var(--ae-line); min-width: 0; }
    .cru-progress > div:nth-child(3n + 1) { border-left: 0; }
    .cru-progress > div:nth-child(-n + 3) { border-top: 0; }
    .cru-json { max-height: 28em; overflow: auto; padding: var(--ae-space-4); background: var(--ae-wash); border: 1px solid var(--ae-line); }
    .cru-toast { position: fixed; right: 1em; bottom: 1em; max-width: 32em; border: 1px solid var(--ae-line); background: var(--ae-surface); padding: .8em 1em; z-index: var(--ae-z-toast); }
    .cru-mobile-bar { display: none; }
    @media (max-width: 820px) {
      .cru-mobile-bar { display: flex; align-items: center; justify-content: space-between; gap: var(--ae-space-3); padding-bottom: var(--ae-space-4); border-bottom: 1px solid var(--ae-line); }
      .cru-mobile-bar .ae-name { margin: 0; }
      .ae-desk { padding: 1em; }
      .cru-grid, .cru-grid.two, .cru-grid.four, .cru-progress { grid-template-columns: 1fr; }
      .cru-progress > div { border-left: 0; }
      .cru-progress > div:nth-child(-n + 3) { border-top: 1px solid var(--ae-line); }
      .cru-progress > div:first-child { border-top: 0; }
    }
  </style>
</head>
<body>
  <div class="ae-shell">
    <aside class="ae-rail">
      <h1 class="ae-logo"><span class="ae-app-mark"><svg class="ae-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2"/><path d="M6.453 15h11.094"/><path d="M8.5 2h7"/></svg></span><span class="ae-name">CRUCIBLE</span></h1>
      <p class="ae-h">BENCHMARK ARENA</p>
      <nav>
        <button data-view-button="benchmarks" aria-current="page">Benchmarks</button>
        <button data-view-button="setup">Run setup</button>
        <button data-view-button="live">Live run</button>
        <button data-view-button="comparison">Comparison</button>
        <button data-view-button="receipts">Receipts</button>
      </nav>
      <div class="ae-rail-foot">
        <button class="ae-mode" type="button" id="mode-toggle" aria-label="toggle color mode">
          <svg class="ae-icon ae-sun" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4v2M12 18v2M4 12h2M18 12h2M6.6 6.6 8 8M16 16l1.4 1.4M17.4 6.6 16 8M8 16l-1.4 1.4" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/><circle cx="12" cy="12" r="3.5" fill="none" stroke="currentColor" stroke-width="1.8"/></svg>
          <svg class="ae-icon ae-moon" viewBox="0 0 24 24" aria-hidden="true"><path d="M17.5 15.8A7 7 0 0 1 8.2 6.5 7.5 7.5 0 1 0 17.5 15.8Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/></svg>
        </button>
      </div>
    </aside>
    <main class="ae-desk cru-desk">
      <div class="cru-mobile-bar">
        <p class="ae-logo"><span class="ae-app-mark"><svg class="ae-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2"/><path d="M6.453 15h11.094"/><path d="M8.5 2h7"/></svg></span><span class="ae-name">CRUCIBLE</span></p>
        <select class="cru-select" id="mobile-view">
          <option value="benchmarks">Benchmarks</option>
          <option value="setup">Run setup</option>
          <option value="live">Live run</option>
          <option value="comparison">Comparison</option>
          <option value="receipts">Receipts</option>
        </select>
      </div>
      <section id="view"></section>
    </main>
  </div>
  <div id="toast" class="cru-toast" hidden></div>
  <script>
    const state = { view: 'benchmarks', specs: null, runs: null, selectedSpecPath: null, selectedRunId: null, detail: null, activeRun: null, comparisonResult: null, filters: {} };
    const view = document.querySelector('#view');
    const toast = document.querySelector('#toast');

    function esc(value) {
      return String(value ?? '').replace(/[&<>"']/g, ch => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
    }
    function pct(value) { return value == null ? 'n/a' : (value * 100).toFixed(1) + '%'; }
    function scoreText(run) { return run?.point == null ? 'No score yet' : `${pct(run.point)} (${run.successes}/${run.n} tasks passed)`; }
    function uncertaintyText(run) { return `uncertainty range ${pct(run.lower)} to ${pct(run.upper)}`; }
    function statusGlyph(ok, warn) {
      const cls = ok ? 'ok' : warn ? 'warn' : 'err';
      const glyph = ok ? '✓' : warn ? '!' : '×';
      return `<span class="cru-status ${cls}"><span class="glyph">${glyph}</span>`;
    }
    function ci(run) {
      const lower = Math.max(0, Math.min(1, run?.lower ?? 0));
      const upper = Math.max(0, Math.min(1, run?.upper ?? 0));
      const point = run?.point == null ? 0 : Math.max(0, Math.min(1, run.point));
      return `<div class="cru-ci" title="${esc(uncertaintyText(run))}">
        <i class="band" style="left:${lower * 100}%;width:${Math.max(1, (upper - lower) * 100)}%"></i>
        <i class="point" style="left:${point * 100}%"></i>
      </div>`;
    }
    function loadJson(url, options) {
      return fetch(url, options).then(async res => {
        const text = await res.text();
        let data;
        try { data = text ? JSON.parse(text) : {}; } catch (_) { data = { error: text }; }
        if (!res.ok) throw new Error(data.error || `${res.status} ${res.statusText}`);
        return data;
      });
    }
    function specs() { return state.specs?.specs || []; }
    function runs() { return state.runs?.runs || []; }
    function selectedSpec() {
      return specs().find(spec => spec.path === state.selectedSpecPath) || specs().find(spec => spec.supports_controlled_comparison) || specs()[0];
    }
    function lastRunFor(spec) {
      return runs().find(run => run.benchmark_id === spec.id);
    }

    async function refreshAll() {
      const params = new URLSearchParams(state.filters);
      const [specsPayload, runsPayload] = await Promise.all([
        loadJson('/api/specs'),
        loadJson('/api/runs' + (params.toString() ? '?' + params : ''))
      ]);
      state.specs = specsPayload;
      state.runs = runsPayload;
      if (!state.selectedSpecPath) {
        const runnable = specs().find(spec => spec.supports_controlled_comparison);
        state.selectedSpecPath = (runnable || specs()[0] || {}).path || null;
      }
      if (!state.selectedRunId && runs()[0]) state.selectedRunId = runs()[0].run_id;
      if (state.selectedRunId) await loadDetail(state.selectedRunId, false).catch(() => {});
      render();
    }
    async function loadDetail(runId, rerender = true) {
      state.selectedRunId = runId;
      state.detail = await loadJson('/api/runs/' + encodeURIComponent(runId));
      if (rerender) render();
    }
    function setView(next) {
      state.view = next;
      document.querySelectorAll('[data-view-button]').forEach(button => {
        if (button.dataset.viewButton === next) button.setAttribute('aria-current', 'page');
        else button.removeAttribute('aria-current');
      });
      document.querySelector('#mobile-view').value = next;
      render();
    }
    function render() {
      if (state.view === 'benchmarks') renderBenchmarks();
      if (state.view === 'setup') renderSetup();
      if (state.view === 'live') renderLive();
      if (state.view === 'comparison') renderComparison();
      if (state.view === 'receipts') renderReceipts();
    }

    function renderBenchmarks() {
      const cards = specs();
      view.innerHTML = `<div class="cru-toolbar">
        <div><p class="cru-title">Benchmark library</p><p class="cru-lede">A benchmark is a set of tasks with deterministic verifiers. Pick one, then compare runners that differ in one declared way.</p></div>
        <button class="cru-button secondary" id="reload" type="button">Refresh</button>
      </div>
      <div class="cru-grid">${cards.map(spec => {
        const last = lastRunFor(spec);
        const selected = selectedSpec()?.path === spec.path;
        const valid = spec.validation?.valid && spec.validation?.runnable;
        return `<article class="cru-card ${selected ? 'selected' : ''}" data-spec-card="${esc(spec.path)}">
          <div class="cru-chipline">
            <span class="cru-chip">${esc(spec.task_count_label)}</span>
            <span class="cru-chip ${valid ? 'ok' : 'err'}">${valid ? 'ready' : 'needs work'}</span>
          </div>
          <p class="cru-title">${esc(spec.benchmark_title || spec.id)}</p>
          <p>${esc(spec.plain_summary)}</p>
          <p><strong>Verifier:</strong> ${esc(spec.verifier_summary)}</p>
          <p><strong>How it runs:</strong> ${esc(spec.runner_summary)}</p>
          <p class="cru-subtle">${last ? `Last result: ${esc(scoreText(last))}; ${esc(uncertaintyText(last))}.` : 'No run in this ledger yet.'}</p>
          <div class="cru-actions">
            <button class="cru-button" data-setup="${esc(spec.path)}" ${spec.supports_controlled_comparison ? '' : 'disabled'} type="button">Set up comparison</button>
            ${last ? `<button class="cru-button secondary" data-run-detail="${esc(last.run_id)}" type="button">Open receipt</button>` : ''}
          </div>
        </article>`;
      }).join('')}</div>
      ${state.specs?.load_errors?.length ? `<div class="cru-empty">${state.specs.load_errors.map(err => esc(err.path + ': ' + err.error)).join('<br>')}</div>` : ''}`;
      document.querySelector('#reload').onclick = refreshAll;
      document.querySelectorAll('[data-spec-card]').forEach(card => card.onclick = () => { state.selectedSpecPath = card.dataset.specCard; renderBenchmarks(); });
      document.querySelectorAll('[data-setup]').forEach(button => button.onclick = event => { event.stopPropagation(); state.selectedSpecPath = button.dataset.setup; setView('setup'); });
      document.querySelectorAll('[data-run-detail]').forEach(button => button.onclick = async event => { event.stopPropagation(); await loadDetail(button.dataset.runDetail, false); setView('receipts'); });
    }

    function renderSetup() {
      const spec = selectedSpec();
      if (!spec) { view.innerHTML = '<div class="cru-empty">No benchmarks found.</div>'; return; }
      const defaults = spec.runner_defaults || {};
      view.innerHTML = `<div class="cru-toolbar">
        <div><p class="cru-title">Run setup</p><p class="cru-lede">Compose two runner bundles. Crucible will show what changed before launch and will label multi-variable comparisons.</p></div>
      </div>
      <section class="cru-card">
        <label class="cru-field"><span class="cru-label">benchmark</span><select class="cru-select" id="spec-select">
          ${specs().map(item => `<option value="${esc(item.path)}" ${item.path === spec.path ? 'selected' : ''}>${esc(item.id)} - ${esc(item.task_count_label)}</option>`).join('')}
        </select></label>
        <p>${esc(spec.plain_summary)}</p>
        <p><strong>Locked verifier:</strong> ${esc(spec.verifier_summary)}</p>
        <p><strong>Locked tool policy:</strong> ${esc(defaults.tool_policy || 'No tool policy declared for this runner.')}</p>
      </section>
      <div class="cru-grid two" style="margin-top: var(--ae-space-4)">
        ${runnerEditor('runner-a', 'Runner A', defaults)}
        ${runnerEditor('runner-b', 'Runner B', { ...defaults, model: alternateModel(defaults.model) })}
      </div>
      <section class="cru-card warning" style="margin-top: var(--ae-space-4)" id="diff-box"></section>
      <div class="cru-actions"><button class="cru-button" id="launch" ${spec.supports_controlled_comparison ? '' : 'disabled'} type="button">Launch controlled comparison</button><button class="cru-button secondary" id="back-library" type="button">Benchmark library</button></div>`;
      document.querySelector('#spec-select').onchange = event => { state.selectedSpecPath = event.target.value; renderSetup(); };
      document.querySelector('#back-library').onclick = () => setView('benchmarks');
      document.querySelectorAll('[data-runner-field]').forEach(input => input.oninput = updateDiffBox);
      document.querySelector('#launch').onclick = launchComparison;
      updateDiffBox();
    }

    function runnerEditor(prefix, title, defaults) {
      return `<section class="cru-card" data-runner="${prefix}">
        <p class="cru-title">${esc(title)}</p>
        <label class="cru-field"><span class="cru-label">runner name</span><input class="cru-input" data-runner-field="${prefix}" name="id" value="${esc(title)}"></label>
        <label class="cru-field"><span class="cru-label">model</span><input class="cru-input" data-runner-field="${prefix}" name="model" value="${esc(defaults.model || '')}"></label>
        <label class="cru-field"><span class="cru-label">system prompt</span><textarea class="cru-textarea" data-runner-field="${prefix}" name="system_prompt">${esc(defaults.system_prompt || '')}</textarea></label>
        <div class="cru-grid two">
          <label class="cru-field"><span class="cru-label">temperature</span><input class="cru-input" data-runner-field="${prefix}" name="temperature" type="number" min="0" step="1" value="${esc(defaults.temperature ?? 0)}"></label>
          <label class="cru-field"><span class="cru-label">max output</span><input class="cru-input" data-runner-field="${prefix}" name="max_output_units" type="number" min="1" step="1" value="${esc(defaults.max_output_units ?? 512)}"></label>
        </div>
      </section>`;
    }
    function alternateModel(model) {
      if (!model) return '';
      return model.includes('deepseek') ? 'z-ai/glm-5.2' : 'deepseek/deepseek-v4-flash';
    }
    function runnerFrom(prefix) {
      const root = document.querySelector(`[data-runner="${prefix}"]`);
      const value = name => root.querySelector(`[name="${name}"]`).value.trim();
      return {
        id: value('id'),
        model: value('model'),
        system_prompt: value('system_prompt'),
        temperature: Number(value('temperature') || 0),
        max_output_units: Number(value('max_output_units') || 512)
      };
    }
    function runnerDiff(a, b) {
      const keys = [['model','model'], ['system_prompt','system prompt'], ['temperature','temperature'], ['max_output_units','max output']];
      return keys.filter(([key]) => String(a[key] ?? '') !== String(b[key] ?? '')).map(([, label]) => label);
    }
    function updateDiffBox() {
      const box = document.querySelector('#diff-box');
      if (!box) return;
      const a = runnerFrom('runner-a');
      const b = runnerFrom('runner-b');
      const diff = runnerDiff(a, b);
      const label = diff.length === 1 ? `Controlled comparison: only ${diff[0]} differs.` : diff.length === 0 ? 'Repeatability check: no runner fields differ.' : `Multi-variable comparison: ${diff.join(', ')} differ.`;
      box.innerHTML = `<p class="cru-title">${esc(label)}</p><p class="cru-subtle">Locked across both runners: benchmark tasks, deterministic verifier, provider boundary, credential source, and tool policy.</p>`;
    }
    async function launchComparison() {
      const spec = selectedSpec();
      const runners = [runnerFrom('runner-a'), runnerFrom('runner-b')];
      const tasks = spec.task_ids?.length ? spec.task_ids : Array.from({ length: spec.task_count || 1 }, (_, index) => `task-${index + 1}`);
      state.activeRun = { status: 'running', spec, runners, tasks, startedAt: new Date().toISOString(), response: null, error: null };
      setView('live');
      try {
        const response = await loadJson('/api/run', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ spec: spec.path, runners })
        });
        state.activeRun = { ...state.activeRun, status: 'complete', response };
        state.comparisonResult = response.comparison || null;
        showToast(response.comparison ? 'Comparison stored and paired.' : 'Runs stored; comparison needs attention.');
        await refreshAll();
        setView('comparison');
      } catch (err) {
        state.activeRun = { ...state.activeRun, status: 'failed', error: err.message };
        renderLive();
        showToast('Run failed: ' + err.message);
      }
    }

    function renderLive() {
      const active = state.activeRun;
      if (!active) { view.innerHTML = '<div class="cru-empty">No active run. Start from Run setup.</div>'; return; }
      const done = active.status === 'complete';
      const failed = active.status === 'failed';
      view.innerHTML = `<div class="cru-toolbar"><div><p class="cru-title">Live run</p><p class="cru-lede">${esc(active.spec.id)} started ${esc(active.startedAt)}. This run appears here immediately; receipts fill in when each runner returns.</p></div></div>
      <section class="cru-card ${failed ? 'warning' : ''}"><p>${statusGlyph(done, !failed)}${failed ? 'failed' : done ? 'complete' : 'running'}</span> ${failed ? esc(active.error) : done ? 'Both runner receipts are stored.' : 'Crucible is executing the runner bundle now.'}</p></section>
      <div class="cru-progress" style="margin-top: var(--ae-space-4)">
        <div class="cru-label">task</div>${active.runners.map(runner => `<div class="cru-label">${esc(runner.id || runner.model)}</div>`).join('')}
        ${active.tasks.map(task => `<div class="cru-code">${esc(task)}</div>${active.runners.map(runner => taskCell(active, runner, task)).join('')}`).join('')}
      </div>`;
    }
    function taskCell(active, runner, taskId) {
      if (!active.response) return `<div>${statusGlyph(false, true)}running</span></div>`;
      const run = (active.response.runs || []).find(row => row.runner_id === runner.id || row.model === runner.model);
      const detail = run?.report?.evals?.[0];
      if (!detail) return `<div>${statusGlyph(false, true)}stored</span></div>`;
      return `<div>${statusGlyph(true, false)}receipt written</span><br><span class="cru-subtle">${esc(scoreText(run))}</span></div>`;
    }

    function renderComparison() {
      const result = state.comparisonResult;
      if (!result) { view.innerHTML = '<div class="cru-empty">No comparison yet. Launch one from Run setup.</div>'; return; }
      const c = result.comparison;
      const left = c.left;
      const right = c.right;
      view.innerHTML = `<div class="cru-toolbar"><div><p class="cru-title">Comparison</p><p class="cru-lede">${esc(result.control_label)} ${esc(result.verdict_explanation)}</p></div><button class="cru-button secondary" id="new-run" type="button">Run again</button></div>
      <div class="cru-grid two">
        ${scoreCard('left', left)}
        ${scoreCard('right', right)}
      </div>
      <section class="cru-card" style="margin-top: var(--ae-space-4)">
        <p class="cru-title">Noise floor verdict</p>
        <p>${esc(result.verdict_explanation)}</p>
        <p class="cru-subtle">Plain English: the uncertainty range shows where the true pass rate could plausibly land for this task sample. If the paired result is inside the noise floor, the measured difference is not strong enough to trust yet.</p>
        <pre class="cru-json cru-code">${esc(JSON.stringify(c.paired || { comparison_kind: c.comparison_kind, note: c.note }, null, 2))}</pre>
      </section>`;
      document.querySelector('#new-run').onclick = () => setView('setup');
    }
    function scoreCard(side, run) {
      return `<section class="cru-card">
        <p class="cru-label">${esc(side)}</p>
        <p class="cru-title">${esc(run.model || run.config_id)}</p>
        <p>${esc(scoreText(run))}</p>
        ${ci(run)}
        <p class="cru-subtle">${esc(uncertaintyText(run))}. This is the range, not a guarantee.</p>
        <p class="cru-code">${esc(run.config_id)}</p>
      </section>`;
    }

    function renderReceipts() {
      const rows = runs();
      const detail = state.detail;
      view.innerHTML = `<div class="cru-toolbar"><div><p class="cru-title">Receipts</p><p class="cru-lede">Stored run records and artifacts. This is the audit trail underneath the plain benchmark view.</p></div></div>
      <div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>benchmark</th><th>score</th><th>runner</th><th>time</th></tr></thead><tbody>
        ${rows.map(run => `<tr class="cru-click" data-run-id="${esc(run.run_id)}"><td class="ae-item">${esc(run.benchmark_id)}</td><td>${esc(scoreText(run))}<br><span class="cru-subtle">${esc(uncertaintyText(run))}</span></td><td class="wrap cru-code">${esc(run.config_id)}<br>${esc(run.model || run.provider || 'deterministic')}</td><td>${esc(new Date(run.created_at_unix_ms).toISOString())}</td></tr>`).join('')}
      </tbody></table></div>
      ${detail ? renderDetail(detail) : ''}`;
      document.querySelectorAll('[data-run-id]').forEach(row => row.onclick = async () => { await loadDetail(row.dataset.runId, true); });
    }
    function renderDetail(detail) {
      const run = detail.run;
      return `<section class="cru-card" style="margin-top: var(--ae-space-4)">
        <p class="cru-title">Run receipt</p><p class="cru-code">${esc(run.run_id)}</p>
        <div class="cru-grid two"><div><p>${esc(scoreText(run))}</p>${ci(run)}<p class="cru-subtle">${esc(uncertaintyText(run))}</p></div><dl class="cru-code"><dt>benchmark</dt><dd>${esc(run.benchmark_id)}</dd><dt>runner</dt><dd>${esc(run.runner_kind)}</dd><dt>report</dt><dd>${esc(run.run_report)}</dd></dl></div>
        <p class="cru-title">Task results</p>${renderTasks(detail)}
        <p class="cru-title">Artifacts</p>${detail.artifacts.length ? `<table class="ae-table"><tbody>${detail.artifacts.map((artifact, index) => `<tr><td>${esc(artifact.kind)}</td><td class="wrap cru-code">${esc(artifact.path)}</td><td><a href="/artifacts/${encodeURIComponent(run.run_id)}/${index}" target="_blank" rel="noreferrer">open</a></td></tr>`).join('')}</tbody></table>` : '<p class="cru-subtle">No artifacts indexed.</p>'}
      </section>`;
    }
    function renderTasks(detail) {
      if (detail.prompt_tasks.length) {
        return `<div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>task</th><th>verdict</th><th>latency</th><th>model</th><th>cost</th></tr></thead><tbody>${detail.prompt_tasks.map(task => `<tr><td class="ae-item">${esc(task.task_id)}</td><td>${statusGlyph(task.passed, false)}${task.passed ? 'pass' : 'fail'}</span></td><td>${task.latency_ms == null ? 'n/a' : esc(task.latency_ms + 'ms')}</td><td>${esc(task.response_model || task.requested_model || '')}</td><td>${task.cost_usd == null ? 'n/a' : '$' + Number(task.cost_usd).toFixed(5)}</td></tr>`).join('')}</tbody></table></div>`;
      }
      if (detail.task_results && detail.task_results.length) {
        return `<div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>task</th><th>trial</th><th>matched</th><th>missed</th><th>false positives</th><th>verifier</th></tr></thead><tbody>${detail.task_results.map(task => `<tr><td class="ae-item">${esc(task.task_id)}</td><td>${esc(task.trial ?? '')}</td><td>${esc(task.matched ?? '')}/${esc(task.expected_defects ?? '')}</td><td>${esc(task.missed ?? '')}</td><td>${esc(task.false_positives ?? '')}</td><td>${statusGlyph(!task.error && !task.scorer_error, false)}${task.error || task.scorer_error ? esc(task.error || task.scorer_error) : 'graded'}</span></td></tr>`).join('')}</tbody></table></div>`;
      }
      return '<p class="cru-subtle">No per-task rows were indexed for this runner.</p>';
    }
    function showToast(message) {
      toast.textContent = message;
      toast.hidden = false;
      clearTimeout(showToast.timer);
      showToast.timer = setTimeout(() => { toast.hidden = true; }, 5000);
    }

    document.querySelectorAll('[data-view-button]').forEach(button => button.onclick = () => setView(button.dataset.viewButton));
    document.querySelector('#mobile-view').onchange = event => setView(event.target.value);
    const root = document.documentElement;
    const savedMode = localStorage.getItem('crucible-mode') || 'light';
    root.setAttribute('data-ae-mode', savedMode);
    document.querySelector('#mode-toggle').onclick = () => {
      const next = root.getAttribute('data-ae-mode') === 'dark' ? 'light' : 'dark';
      root.setAttribute('data-ae-mode', next);
      localStorage.setItem('crucible-mode', next);
    };
    refreshAll().catch(err => {
      view.innerHTML = `<div class="cru-empty">Load failed: ${esc(err.message)}</div>`;
    });
  </script>
</body>
</html>
"#
    .to_string()
}
