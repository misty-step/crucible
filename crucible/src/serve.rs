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
    let corpus = spec
        .runner
        .as_ref()
        .map(|runner| corpus_summary(&runner.corpus))
        .unwrap_or_else(|| "definition_only".to_string());
    SpecSummary {
        path: display_path(&path),
        id: spec.id,
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
}

#[derive(Debug, Serialize)]
struct RunSpecResponse {
    schema_version: &'static str,
    spec: String,
    output_dir: String,
    stored: run_store::PersistedReport,
    report: crate::eval_run::RunReport,
}

fn run_spec_response(db_path: &Path, specs_dir: &Path, body: &[u8]) -> Result<RunSpecResponse> {
    let request: RunSpecRequest =
        serde_json::from_slice(body).context("parsing run request JSON body")?;
    let spec_path = resolve_requested_spec(specs_dir, &request.spec)?;
    let out_dir = request
        .out
        .map(PathBuf::from)
        .unwrap_or_else(|| default_run_out(&spec_path));
    let report = spec_run::run(&spec_path, Some(&out_dir))?;
    let stored = run_store::persist_report(db_path, &report)?;
    Ok(RunSpecResponse {
        schema_version: RUN_ACTION_SCHEMA,
        spec: display_path(&spec_path),
        output_dir: report.output_dir.clone(),
        stored,
        report,
    })
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

fn display_path(path: &Path) -> String {
    path.display().to_string()
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
  <title>Crucible</title>
  <link rel="stylesheet" href="/assets/aesthetic.css">
  <style>
    :root { --ae-accent: #8a3b30; --ae-accent-dark: #ff9f90; }
    .cru-desk { display: grid; gap: var(--ae-space-5); align-content: start; }
    .cru-toolbar { display: flex; gap: var(--ae-space-3); align-items: center; justify-content: space-between; flex-wrap: wrap; }
    .cru-title { font-weight: var(--ae-w-black); }
    .cru-subtle { color: var(--ae-ink-muted); }
    .cru-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--ae-space-4); }
    .cru-grid.two { grid-template-columns: 1fr 1fr; }
    .cru-filters { display: grid; grid-template-columns: repeat(5, minmax(7em, 1fr)) 7em; gap: var(--ae-space-3); align-items: end; }
    .cru-field { display: grid; gap: var(--ae-space-1); }
    .cru-field span { font-family: var(--ae-font-mono); font-size: 13px; color: var(--ae-ink-muted); }
    .cru-input { appearance: none; border: 1px solid var(--ae-line); background: var(--ae-surface); color: var(--ae-ink); padding: .55em .65em; border-radius: 0; min-width: 0; }
    .cru-button { appearance: none; border: 1px solid var(--ae-ink); background: var(--ae-ink); color: var(--ae-surface); padding: .55em .8em; border-radius: 0; cursor: pointer; }
    .cru-button.secondary { background: transparent; color: var(--ae-ink); border-color: var(--ae-line); }
    .cru-button:disabled { opacity: .45; cursor: default; }
    .cru-table-wrap { overflow: auto; border: 1px solid var(--ae-line); }
    .cru-table-wrap .ae-table th, .cru-table-wrap .ae-table td { white-space: nowrap; vertical-align: top; }
    .cru-table-wrap .ae-table td.wrap { white-space: normal; min-width: 12em; }
    .cru-table-wrap .ae-table td.tight { max-width: 18em; overflow-wrap: anywhere; white-space: normal; }
    .cru-click { cursor: pointer; }
    .cru-click:hover td { background: var(--ae-wash); }
    .cru-status { display: inline-flex; gap: .35em; align-items: baseline; }
    .cru-status.ok .glyph { color: var(--ae-ok); }
    .cru-status.warn .glyph { color: var(--ae-warn); }
    .cru-status.err .glyph { color: var(--ae-err); }
    .cru-ci { position: relative; height: 1.5em; border-bottom: 1px solid var(--ae-line); margin-top: .4em; }
    .cru-ci .band { position: absolute; top: .58em; height: 4px; background: var(--ae-wash); border: 1px solid var(--ae-line); }
    .cru-ci .point { position: absolute; top: .25em; width: 1px; height: 1em; background: var(--ae-ink); }
    .cru-code { font-family: var(--ae-font-mono); font-size: 13px; word-break: break-word; }
    .cru-empty { border: 1px solid var(--ae-line); padding: var(--ae-space-5); color: var(--ae-ink-muted); }
    .cru-detail { display: grid; grid-template-columns: minmax(16em, .75fr) minmax(0, 1.25fr); gap: var(--ae-space-4); align-items: start; }
    .cru-json { max-height: 32em; overflow: auto; padding: var(--ae-space-4); background: var(--ae-wash); border: 1px solid var(--ae-line); }
    .cru-toast { position: fixed; right: 1em; bottom: 1em; max-width: 32em; border: 1px solid var(--ae-line); background: var(--ae-surface); padding: .8em 1em; z-index: var(--ae-z-toast); }
    .cru-mobile-bar { display: none; }
    @media (max-width: 640px) {
      .cru-mobile-bar { display: flex; align-items: center; justify-content: space-between; gap: var(--ae-space-3); padding-bottom: var(--ae-space-4); border-bottom: 1px solid var(--ae-line); }
      .cru-mobile-bar .ae-name { margin: 0; }
      .ae-desk { padding: 1em; }
      .cru-grid, .cru-grid.two, .cru-detail, .cru-filters { grid-template-columns: 1fr; }
    }
  </style>
</head>
<body>
  <div class="ae-shell">
    <aside class="ae-rail">
      <h1 class="ae-name">CRUCIBLE</h1>
      <p class="ae-h">VIEWS</p>
      <nav>
        <button data-view-button="specs" aria-current="page">Specs</button>
        <button data-view-button="runs">Runs</button>
        <button data-view-button="detail">Run Detail</button>
        <button data-view-button="adjudicate">Adjudicate</button>
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
        <p class="ae-name">CRUCIBLE</p>
        <select class="cru-input" id="mobile-view">
          <option value="specs">Specs</option>
          <option value="runs">Runs</option>
          <option value="detail">Run Detail</option>
          <option value="adjudicate">Adjudicate</option>
        </select>
      </div>
      <section id="view"></section>
    </main>
  </div>
  <div id="toast" class="cru-toast" hidden></div>
  <script>
    const state = { view: 'specs', specs: null, runs: null, detail: null, adjudication: null, selectedRunId: null, filters: {} };
    const view = document.querySelector('#view');
    const toast = document.querySelector('#toast');

    function esc(value) {
      return String(value ?? '').replace(/[&<>"']/g, ch => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
    }

    function pct(value) {
      return value == null ? 'n/a' : (value * 100).toFixed(1) + '%';
    }

    function ci(run) {
      const lower = Math.max(0, Math.min(1, run.lower ?? 0));
      const upper = Math.max(0, Math.min(1, run.upper ?? 0));
      const point = run.point == null ? 0 : Math.max(0, Math.min(1, run.point));
      return `<div class="cru-ci" aria-label="Wilson interval ${pct(run.lower)} to ${pct(run.upper)}">
        <i class="band" style="left:${lower * 100}%;width:${Math.max(1, (upper - lower) * 100)}%"></i>
        <i class="point" style="left:${point * 100}%"></i>
      </div>`;
    }

    function statusGlyph(ok, warn) {
      const cls = ok ? 'ok' : warn ? 'warn' : 'err';
      const glyph = ok ? '✓' : warn ? '!' : '×';
      return `<span class="cru-status ${cls}"><span class="glyph">${glyph}</span>`;
    }

    function spark(points) {
      if (!points || points.length === 0) return '';
      const vals = points.map(p => p.point == null ? 0 : p.point);
      const min = Math.min(...vals, 0);
      const max = Math.max(...vals, 1);
      const span = Math.max(.001, max - min);
      const coords = vals.map((v, i) => {
        const x = points.length === 1 ? 50 : (i / (points.length - 1)) * 100;
        const y = 22 - ((v - min) / span) * 20;
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      }).join(' ');
      return `<svg class="ae-spark" viewBox="0 0 100 24" preserveAspectRatio="none" aria-hidden="true"><polyline points="${coords}"></polyline></svg>`;
    }

    async function loadJson(url, options) {
      const res = await fetch(url, options);
      const text = await res.text();
      let data;
      try { data = text ? JSON.parse(text) : {}; } catch (_) { data = { error: text }; }
      if (!res.ok) throw new Error(data.error || `${res.status} ${res.statusText}`);
      return data;
    }

    async function refreshAll() {
      const params = new URLSearchParams(state.filters);
      const [specs, runs, adjudication] = await Promise.all([
        loadJson('/api/specs'),
        loadJson('/api/runs' + (params.toString() ? '?' + params : '')),
        loadJson('/api/adjudication')
      ]);
      state.specs = specs;
      state.runs = runs;
      state.adjudication = adjudication;
      if (!state.selectedRunId && runs.runs[0]) state.selectedRunId = runs.runs[0].run_id;
      if (state.selectedRunId) await loadDetail(state.selectedRunId, false);
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
      if (state.view === 'specs') renderSpecs();
      if (state.view === 'runs') renderRuns();
      if (state.view === 'detail') renderDetail();
      if (state.view === 'adjudicate') renderAdjudicate();
    }

    function renderSpecs() {
      const specs = state.specs?.specs || [];
      view.innerHTML = `<div class="cru-toolbar">
        <div><p class="cru-title">Benchmark library</p><p class="cru-subtle">Declared EvalSpecs with live validate output.</p></div>
        <button class="cru-button secondary" id="reload-specs" type="button">Refresh</button>
      </div>
      <div class="cru-table-wrap"><table class="ae-table">
        <thead><tr><th>spec</th><th>measures</th><th>graders</th><th>validate</th></tr></thead>
        <tbody>${specs.map(spec => `<tr>
          <td class="ae-item tight">${esc(spec.id)}<br><button class="cru-button secondary" data-run-spec="${esc(spec.path)}" type="button">Run</button></td>
          <td class="wrap">${esc(spec.inputs)}<br><span class="cru-subtle">${esc(spec.outputs)}</span></td>
          <td class="tight">${spec.graders.map(g => esc(g.kind + ':' + g.id)).join('<br>') || 'none'}</td>
          <td>${statusGlyph(spec.validation.valid, spec.validation.warnings.length > 0)}${spec.validation.runnable ? 'runnable' : 'not runnable'}</span><br><span class="cru-subtle">${pct(spec.confidence)} CI, ${spec.validation.errors.length} errors, ${spec.validation.warnings.length} warnings</span></td>
        </tr>`).join('')}</tbody>
      </table></div>
      ${state.specs?.load_errors?.length ? `<div class="cru-empty">${state.specs.load_errors.map(err => esc(err.path + ': ' + err.error)).join('<br>')}</div>` : ''}`;
      document.querySelector('#reload-specs').onclick = refreshAll;
      document.querySelectorAll('[data-run-spec]').forEach(button => {
        button.onclick = () => runSpec(button.dataset.runSpec, button);
      });
    }

    async function runSpec(path, button) {
      button.disabled = true;
      showToast('Running ' + path + '...');
      try {
        const response = await loadJson('/api/run', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ spec: path })
        });
        showToast('Run stored: ' + response.stored.invocation_id);
        await refreshAll();
        const first = response.report.evals?.[0]?.id;
        const run = state.runs.runs.find(row => row.invocation_id === response.stored.invocation_id && (!first || row.benchmark_id === first));
        if (run) { await loadDetail(run.run_id, false); setView('detail'); }
      } catch (err) {
        showToast('Run failed: ' + err.message);
      } finally {
        button.disabled = false;
      }
    }

    function renderRuns() {
      const runs = state.runs?.runs || [];
      const trends = new Map((state.runs?.trendlines || []).map(t => [t.benchmark_id, t.points]));
      view.innerHTML = `<div class="cru-toolbar">
        <div><p class="cru-title">Runs ledger</p><p class="cru-subtle">Filterable run records over time, with score trendlines per benchmark.</p></div>
      </div>
      <form class="cru-filters" id="filters">
        ${['benchmark','config','model','since','until'].map(key => `<label class="cru-field"><span>${key}</span><input class="cru-input" name="${key}" value="${esc(state.filters[key] || '')}"></label>`).join('')}
        <button class="cru-button" type="submit">Apply</button>
      </form>
      <div class="cru-table-wrap"><table class="ae-table">
        <thead><tr><th>spec</th><th>trend</th><th class="num">score</th><th>config / model</th><th>run</th></tr></thead>
        <tbody>${runs.map(run => `<tr class="cru-click" data-run-id="${esc(run.run_id)}">
          <td class="ae-item tight">${esc(run.benchmark_id)}<br><span class="cru-subtle">${esc(run.score_metric)}</span></td>
          <td>${spark(trends.get(run.benchmark_id))}</td>
          <td class="num">${pct(run.point)}<br><span class="cru-subtle">${pct(run.lower)}-${pct(run.upper)}</span></td>
          <td class="tight">${esc(run.config_id)}<br><span class="cru-subtle">${esc(run.model || run.provider || 'deterministic')}</span></td>
          <td class="tight">${esc(run.run_id)}<br><span class="cru-subtle">${esc(new Date(run.created_at_unix_ms).toISOString())}</span></td>
        </tr>`).join('')}</tbody>
      </table></div>`;
      document.querySelector('#filters').onsubmit = async event => {
        event.preventDefault();
        const form = new FormData(event.currentTarget);
        state.filters = {};
        for (const [key, value] of form.entries()) if (String(value).trim()) state.filters[key] = String(value).trim();
        await refreshAll();
      };
      document.querySelectorAll('[data-run-id]').forEach(row => {
        row.onclick = async () => { await loadDetail(row.dataset.runId, false); setView('detail'); };
      });
    }

    function renderDetail() {
      const detail = state.detail;
      if (!detail) {
        view.innerHTML = '<div class="cru-empty">No run selected.</div>';
        return;
      }
      const run = detail.run;
      const adjudication = adjudicationStatus(detail);
      view.innerHTML = `<div class="cru-toolbar">
        <div><p class="cru-title">Run receipt</p><p class="cru-subtle cru-code">${esc(run.run_id)}</p></div>
        <button class="cru-button secondary" id="back-runs" type="button">Runs</button>
      </div>
      <div class="cru-detail">
        <section class="ae-plate">
          <p class="ae-plate-cap">SCORE</p>
          <p><span class="ae-strong">${pct(run.point)}</span> ${esc(run.method)} ${pct(run.confidence)} CI [${pct(run.lower)}, ${pct(run.upper)}]</p>
          ${ci(run)}
          <p class="cru-subtle">${esc(run.successes)} / ${esc(run.n)} successes on ${esc(run.score_metric)}</p>
          <p>${statusGlyph(adjudication.ok, adjudication.warn)}${esc(adjudication.label)}</span></p>
        </section>
        <section class="ae-plate">
          <p class="ae-plate-cap">REPRODUCIBILITY</p>
          <dl class="cru-code">
            <dt>benchmark</dt><dd>${esc(run.benchmark_id)}</dd>
            <dt>runner</dt><dd>${esc(run.runner_kind)}</dd>
            <dt>config</dt><dd>${esc(run.config_id)}</dd>
            <dt>model</dt><dd>${esc(run.model || run.provider || 'deterministic')}</dd>
            <dt>report</dt><dd>${esc(run.run_report)}</dd>
          </dl>
        </section>
      </div>
      <section class="ae-plate">
        <p class="ae-plate-cap">PER-TRIAL / TASK RESULTS</p>
        ${renderTasks(detail)}
      </section>
      <section class="ae-plate">
        <p class="ae-plate-cap">ARTIFACTS</p>
        ${detail.artifacts.length ? `<table class="ae-table"><tbody>${detail.artifacts.map((artifact, index) => `<tr><td>${esc(artifact.kind)}</td><td class="wrap cru-code">${esc(artifact.path)}</td><td><a href="/artifacts/${encodeURIComponent(run.run_id)}/${index}" target="_blank" rel="noreferrer">open</a></td></tr>`).join('')}</tbody></table>` : '<p class="cru-subtle">No artifacts indexed.</p>'}
      </section>
      <section class="ae-plate">
        <p class="ae-plate-cap">EVAL JSON</p>
        <pre class="cru-json cru-code">${esc(JSON.stringify(detail.eval_json, null, 2))}</pre>
      </section>`;
      document.querySelector('#back-runs').onclick = () => setView('runs');
    }

    function renderTasks(detail) {
      if (detail.prompt_tasks.length) {
        return `<div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>task</th><th>verdict</th><th>latency</th><th>model</th><th>cost</th></tr></thead><tbody>
          ${detail.prompt_tasks.map(task => `<tr>
            <td class="ae-item">${esc(task.task_id)}</td>
            <td>${statusGlyph(task.passed, false)}${task.passed ? 'pass' : 'fail'}</span></td>
            <td>${task.latency_ms == null ? 'n/a' : esc(task.latency_ms + 'ms')}</td>
            <td>${esc(task.response_model || task.requested_model || '')}</td>
            <td>${task.cost_usd == null ? 'n/a' : '$' + Number(task.cost_usd).toFixed(5)}</td>
          </tr>`).join('')}
        </tbody></table></div>`;
      }
      if (detail.task_results && detail.task_results.length) {
        return `<div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>task</th><th>trial</th><th class="num">matched</th><th class="num">missed</th><th class="num">false +</th><th>grader</th></tr></thead><tbody>
          ${detail.task_results.map(task => `<tr>
            <td class="ae-item tight">${esc(task.task_id)}<br><span class="cru-subtle">${esc(task.run_id || '')}</span></td>
            <td class="num">${esc(task.trial ?? '')}</td>
            <td class="num">${esc(task.matched ?? '')}/${esc(task.expected_defects ?? '')}</td>
            <td class="num">${esc(task.missed ?? '')}</td>
            <td class="num">${esc(task.false_positives ?? '')}</td>
            <td>${statusGlyph(!task.error && !task.scorer_error, false)}${task.error || task.scorer_error ? esc(task.error || task.scorer_error) : 'graded'}</span></td>
          </tr>`).join('')}
        </tbody></table></div>`;
      }
      const notes = detail.eval_json?.notes || [];
      return `<p class="cru-subtle">No indexed prompt-task rows for this runner. The receipt below is the durable grader result for this run.</p>
        ${notes.length ? `<ul>${notes.map(note => `<li>${esc(note)}</li>`).join('')}</ul>` : ''}`;
    }

    function adjudicationStatus(detail) {
      if (detail.adjudication_status === 'labels_present') return { ok: true, warn: false, label: 'adjudication labels present' };
      if (detail.adjudication_status === 'queue_present') return { ok: false, warn: true, label: 'adjudication queue artifact present' };
      return { ok: false, warn: false, label: 'no adjudication queue indexed' };
    }

    function renderAdjudicate() {
      const panels = state.adjudication?.panels || [];
      view.innerHTML = `<div class="cru-toolbar"><div><p class="cru-title">Adjudicate</p><p class="cru-subtle">Existing adjudication panel artifacts linked from run receipts.</p></div></div>
      ${panels.length ? `<div class="cru-grid">${panels.map(panel => `<section class="ae-plate">
        <p class="ae-plate-cap">${esc(panel.benchmark_id)}</p>
        <p class="cru-code">${esc(panel.run_id)}</p>
        <p>${esc(panel.title)}</p>
        <p>${panel.panel_url ? `<a href="${esc(panel.panel_url)}" target="_blank" rel="noreferrer">Open existing panel</a>` : 'No panel html artifact'}</p>
        <p class="cru-subtle cru-code">${esc(panel.queue_path || '')}</p>
      </section>`).join('')}</div>` : '<div class="cru-empty">No adjudication queue artifacts are indexed in this ledger.</div>'}`;
    }

    function showToast(message) {
      toast.textContent = message;
      toast.hidden = false;
      clearTimeout(showToast.timer);
      showToast.timer = setTimeout(() => { toast.hidden = true; }, 5000);
    }

    document.querySelectorAll('[data-view-button]').forEach(button => {
      button.onclick = () => setView(button.dataset.viewButton);
    });
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
