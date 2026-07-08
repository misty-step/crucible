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
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use crucible_core::{CorpusSpec, EvalSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{adjudication_panel, adjudication_server, load_queue, run_store, spec_run, validate};

const SPECS_SCHEMA: &str = "crucible.ui.specs.v1";
const SPEC_DETAIL_SCHEMA: &str = "crucible.ui.spec_detail.v1";
const MATRIX_SCHEMA: &str = "crucible.ui.eval_matrix.v1";
const RUNS_SCHEMA: &str = "crucible.ui.runs.v1";
const ADJUDICATION_SCHEMA: &str = "crucible.ui.adjudication.v1";
const RUN_ACTION_SCHEMA: &str = "crucible.ui.run_action.v1";
const RUN_COMPARISON_SCHEMA: &str = "crucible.ui.run_comparison.v1";
const SERVE_TOKEN_ENV: &str = "CRUCIBLE_SERVE_TOKEN";
/// Opt-out of the bearer gate for deployments that sit behind a trusted network
/// layer (a Tailscale-private box, an authenticated reverse proxy). When set to
/// a truthy value the operator is asserting that the front — not this process —
/// is the access control, exactly as the Sanctum artifact shelf treats tailnet
/// membership. Unset (the default) keeps the fail-closed bearer gate, so a bare
/// `crucible serve` on a laptop is never silently unauthenticated.
const SERVE_TRUST_NETWORK_ENV: &str = "CRUCIBLE_SERVE_TRUST_NETWORK";
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
    if env_flag(SERVE_TRUST_NETWORK_ENV) {
        println!(
            "crucible serve: {SERVE_TRUST_NETWORK_ENV} is set — the bearer gate is OFF; \
             expose this ONLY behind a trusted network (Tailscale-private box or \
             authenticated reverse proxy), never a public endpoint"
        );
    } else if std::env::var(SERVE_TOKEN_ENV)
        .map(|token| token.trim().is_empty())
        .unwrap_or(true)
    {
        println!(
            "crucible serve: same-origin mode — no {SERVE_TOKEN_ENV} set; this UI and \
             local CLI/agent calls work without auth, foreign browser origins are \
             refused (403). Set {SERVE_TOKEN_ENV} to require a bearer token."
        );
    }
    std::io::stdout().flush().ok();

    // One thread per connection: a slow, stuck, or merely chatty viewer must
    // not stall every other request behind it in the accept loop. This is a
    // localhost-only, single-operator dev workbench (bearer-gated protected
    // routes, unless CRUCIBLE_SERVE_TRUST_NETWORK opts out behind a trusted
    // front), not an internet-facing service under load, so unbounded
    // thread-per-connection is the right amount of complexity — a pooled or
    // async design would be solving a load problem this server doesn't have.
    let opts = Arc::new(opts);
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("crucible serve: accept error: {err:#}");
                continue;
            }
        };
        let opts = Arc::clone(&opts);
        thread::spawn(move || {
            if let Err(err) = handle_connection(stream, &opts) {
                eprintln!("crucible serve: connection error: {err:#}");
            }
        });
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
        ("GET", "/api/spec") => match spec_detail_response(&opts.specs_dir, &request.query) {
            Ok(response) => HttpResponse::json_ok(&response),
            Err(err) if is_spec_detail_request_error(&err) => Ok(HttpResponse::json(
                400,
                &json!({ "error": err.to_string() }),
            )),
            Err(err) => Err(err),
        },
        ("GET", "/api/runs") => protected(request, || {
            HttpResponse::json_ok(&runs_response(&opts.db_path, &request.query)?)
        }),
        ("GET", "/api/matrix") => protected(request, || {
            match matrix_query_response(&opts.db_path, &request.query) {
                Ok(response) => HttpResponse::json_ok(&response),
                Err(err) if is_matrix_request_error(&err) => Ok(HttpResponse::json(
                    400,
                    &json!({ "error": err.to_string() }),
                )),
                Err(err) => Err(err),
            }
        }),
        ("GET", "/api/adjudication") => protected(request, || {
            HttpResponse::json_ok(&adjudication_response(&opts.db_path)?)
        }),
        ("GET", "/api/compare") => protected(request, || {
            match compare_query_response(&opts.db_path, &request.query) {
                Ok(response) => HttpResponse::json_ok(&response),
                Err(err) if is_compare_request_error(&err) => Ok(HttpResponse::json(
                    400,
                    &json!({ "error": err.to_string() }),
                )),
                Err(err) => Err(err),
            }
        }),
        ("GET", "/api/history") => protected(request, || {
            match history_query_response(&opts.db_path, &request.query) {
                Ok(response) => HttpResponse::json_ok(&response),
                Err(err) if is_history_request_error(&err) => Ok(HttpResponse::json(
                    400,
                    &json!({ "error": err.to_string() }),
                )),
                Err(err) => Err(err),
            }
        }),
        ("GET", "/api/pivot") => protected(request, || {
            match pivot_query_response(&opts.db_path, &request.query) {
                Ok(response) => HttpResponse::json_ok(&response),
                Err(err) if is_pivot_request_error(&err) => Ok(HttpResponse::json(
                    400,
                    &json!({ "error": err.to_string() }),
                )),
                Err(err) => Err(err),
            }
        }),
        ("POST", "/api/run") => protected(request, || {
            match run_spec_response(&opts.db_path, &opts.specs_dir, &request.body) {
                Ok(response) => HttpResponse::json_ok(&response),
                Err(err) if is_run_request_error(&err) => Ok(HttpResponse::json(
                    400,
                    &json!({ "error": err.to_string() }),
                )),
                Err(err) => Err(err),
            }
        }),
        ("GET", path) if path.starts_with("/api/runs/") => protected(request, || {
            let raw = path.trim_start_matches("/api/runs/");
            let run_id = percent_decode(raw)?;
            HttpResponse::json_ok(&run_detail_response(&opts.db_path, &run_id)?)
        }),
        ("GET", path) if path.starts_with("/adjudication/panel/") => {
            protected(request, || serve_adjudication_panel(path, &opts.db_path))
        }
        ("POST", path) if path.starts_with("/adjudication/panel/") && path.ends_with("/label") => {
            protected(request, || {
                match submit_adjudication_label(path, &opts.db_path, &request.body) {
                    Ok(response) => Ok(response),
                    Err(err) if is_label_request_error(&err) => Ok(HttpResponse::json(
                        400,
                        &json!({ "error": err.to_string() }),
                    )),
                    Err(err) => Err(err),
                }
            })
        }
        ("GET", path) if path.starts_with("/artifacts/") => {
            protected(request, || serve_artifact(path, &opts.db_path))
        }
        _ => Ok(HttpResponse::text(404, "not found")),
    }
}

struct HttpRequest {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
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
        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).context("reading header")?;
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some((name, value)) = trimmed.split_once(':') {
                headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
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
            headers,
            body,
        })
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
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
            401 => "Unauthorized",
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

fn protected(
    request: &HttpRequest,
    handle: impl FnOnce() -> Result<HttpResponse>,
) -> Result<HttpResponse> {
    match require_bearer_auth(request) {
        Ok(()) => handle(),
        Err(response) => Ok(response),
    }
}

fn require_bearer_auth(request: &HttpRequest) -> std::result::Result<(), HttpResponse> {
    let token = std::env::var(SERVE_TOKEN_ENV).ok();
    let token = token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    authorize(
        env_flag(SERVE_TRUST_NETWORK_ENV),
        token,
        request.header("authorization"),
        request.header("origin"),
        request.header("host"),
    )
    .map_err(|deny| match deny {
        Deny::Unauthorized(message) => auth_error(message),
        Deny::Forbidden(message) => forbidden_error(message),
    })
}

/// Why a protected request was refused: `Unauthorized` (401 + bearer hint) when
/// a configured token was missing/wrong; `Forbidden` (403) when a browser's
/// cross-origin request was refused in same-origin mode.
#[derive(Debug, PartialEq)]
enum Deny {
    Unauthorized(&'static str),
    Forbidden(&'static str),
}

/// The auth decision as a pure function, so it is unit-testable without
/// mutating the process-global auth env vars. Three modes:
///
/// - `trust_network`: the operator asserted the network layer (tailnet,
///   authenticated proxy) is the access control — everything passes.
/// - token configured: bearer required, exactly as before. The token remains
///   the only defense that also covers *non-browser* local processes.
/// - neither (the default, "same-origin mode"): requests with no `Origin`
///   header (curl, CLIs, agents, same-origin GETs) and requests whose `Origin`
///   matches the request `Host` (this UI in a browser) pass; a foreign
///   `Origin` is refused. Browsers always stamp the real page origin on
///   cross-site requests and scripts cannot forge it, so this kills the
///   drive-by-webpage CSRF vector against localhost — the attack that makes a
///   spend-capable `POST /api/run` dangerous to leave open — without any
///   token prompt for the operator.
fn authorize(
    trust_network: bool,
    expected_token: Option<&str>,
    auth_header: Option<&str>,
    origin: Option<&str>,
    host: Option<&str>,
) -> std::result::Result<(), Deny> {
    if trust_network {
        return Ok(());
    }
    if let Some(expected) = expected_token {
        let Some(header) = auth_header else {
            return Err(Deny::Unauthorized("authorization bearer token required"));
        };
        let Some(actual) = header.strip_prefix("Bearer ") else {
            return Err(Deny::Unauthorized("authorization bearer token required"));
        };
        return if constant_time_eq(actual.as_bytes(), expected.as_bytes()) {
            Ok(())
        } else {
            Err(Deny::Unauthorized("authorization bearer token required"))
        };
    }
    match origin {
        None => Ok(()),
        Some(origin) if origin_matches_host(origin, host) => Ok(()),
        Some(_) => Err(Deny::Forbidden(
            "cross-origin request refused; set CRUCIBLE_SERVE_TOKEN for cross-origin API access",
        )),
    }
}

/// Does a browser `Origin` header name this server? Compares the origin's
/// authority (scheme stripped) against the request's `Host` header. An absent
/// or opaque (`null`) origin never matches — sandboxed/redirect contexts are
/// treated as foreign.
fn origin_matches_host(origin: &str, host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    let authority = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .unwrap_or(origin);
    !authority.is_empty()
        && authority
            .trim_end_matches('/')
            .eq_ignore_ascii_case(host.trim())
}

/// A permissive truthy-env check (`1`/`true`/`yes`/`on`, case-insensitive) for
/// the trust-network opt-out. Anything else — including unset — reads as false,
/// so the gate stays fail-closed by default.
fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .ok()
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

fn auth_error(message: &str) -> HttpResponse {
    HttpResponse::json(
        401,
        &json!({
            "error": message,
            "auth": "bearer",
            "env": SERVE_TOKEN_ENV
        }),
    )
}

fn forbidden_error(message: &str) -> HttpResponse {
    HttpResponse::json(403, &json!({ "error": message }))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max {
        diff |= usize::from(*left.get(index).unwrap_or(&0) ^ *right.get(index).unwrap_or(&0));
    }
    diff == 0
}

fn is_run_request_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("run output path")
        || message.contains("known spec")
        || message.contains("parsing run request JSON body")
}

const COMPARE_SCHEMA: &str = "crucible.ui.compare.v1";

/// The `GET /api/compare` response: the same `ConfigComparison` the CLI's
/// `runs compare` and the MCP `crucible_runs_compare` tool return, plus the
/// findings journal computed from it — the local API-face analog of both.
#[derive(Debug, Serialize)]
struct CompareResponse {
    schema_version: &'static str,
    comparison: run_store::ConfigComparison,
    findings_journal: crate::findings_journal::FindingsJournal,
}

fn is_compare_request_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("query param")
}

/// `?benchmark=&left=&right=&alpha=` over the server's configured run
/// ledger — no new runs are launched, unlike `POST /api/run`.
fn compare_query_response(
    db_path: &Path,
    query: &HashMap<String, String>,
) -> Result<CompareResponse> {
    let benchmark = query
        .get("benchmark")
        .filter(|value| !value.is_empty())
        .context("missing benchmark query param")?;
    let left = query
        .get("left")
        .filter(|value| !value.is_empty())
        .context("missing left query param")?;
    let right = query
        .get("right")
        .filter(|value| !value.is_empty())
        .context("missing right query param")?;
    let alpha = match query.get("alpha") {
        Some(value) => value
            .parse::<f64>()
            .with_context(|| format!("invalid alpha query param {value:?}"))?,
        None => run_store::DEFAULT_ALPHA,
    };

    // The dashboard is a read-only view: never refuse a multi-axis
    // comparison outright (backlog 974's `strict`), always render it with
    // its `attribution`/`attribution_note` caveat visible instead.
    let comparison = run_store::compare_configs(db_path, benchmark, left, right, alpha, false)?;
    let findings_journal =
        findings_journal_for(db_path, benchmark, left, right, alpha, &comparison);
    Ok(CompareResponse {
        schema_version: COMPARE_SCHEMA,
        comparison,
        findings_journal,
    })
}

fn is_history_request_error(err: &anyhow::Error) -> bool {
    err.to_string().contains("query param")
}

/// `?benchmark=&config=` over the server's configured run ledger — the same
/// time-series `run_store::score_history` and `crucible runs history` return
/// (backlog 027).
fn history_query_response(
    db_path: &Path,
    query: &HashMap<String, String>,
) -> Result<run_store::ScoreHistory> {
    let benchmark = query
        .get("benchmark")
        .filter(|value| !value.is_empty())
        .context("missing benchmark query param")?;
    let config = query
        .get("config")
        .filter(|value| !value.is_empty())
        .context("missing config query param")?;
    run_store::score_history(db_path, benchmark, config)
}

fn is_pivot_request_error(err: &anyhow::Error) -> bool {
    err.to_string().contains("query param")
}

/// `?benchmark=&harness=` over the server's configured run ledger — the same
/// cross-axis pivot `run_store::pivot_by_model` and `crucible runs pivot`
/// return (backlog 027). `harness` is optional; omitting it pivots across
/// every harness recorded for the benchmark.
fn pivot_query_response(
    db_path: &Path,
    query: &HashMap<String, String>,
) -> Result<run_store::PivotView> {
    let benchmark = query
        .get("benchmark")
        .filter(|value| !value.is_empty())
        .context("missing benchmark query param")?;
    let harness = query.get("harness").filter(|value| !value.is_empty());
    run_store::pivot_by_model(db_path, benchmark, harness.map(String::as_str))
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

fn is_spec_detail_request_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("query param") || message.contains("no eval spec found")
}

/// `GET /api/spec?id=<eval id>` — the eval-detail hub's task drill-down
/// source: the full `EvalSpec` (via the existing `SpecSummary` projection) plus,
/// for a `prompt_benchmark` corpus, every task's prompt text, resolved context
/// file content, and expectation — the declared definition half of the
/// drill-down (the other half, every run's actual response, comes from
/// `/api/matrix`'s cells so the client never has to reconcile two different
/// task orderings). Unprotected like `/api/specs`: this is eval *definition*
/// data (declared prompts/rubrics already committed to the repo), not run
/// output.
fn spec_detail_response(
    specs_dir: &Path,
    query: &HashMap<String, String>,
) -> Result<SpecDetailResponse> {
    let id = query
        .get("id")
        .filter(|value| !value.is_empty())
        .context("missing id query param")?;
    let mut paths = json_files(specs_dir)?;
    paths.sort();
    for path in paths {
        let (Ok(validation), Ok(spec)) = (validate::validate(&path), spec_run::load_spec(&path))
        else {
            continue;
        };
        if spec.id != *id {
            continue;
        }
        let prompt_tasks = spec_task_details(&path, &spec);
        let summary = spec_summary(path, spec, validation);
        return Ok(SpecDetailResponse {
            schema_version: SPEC_DETAIL_SCHEMA,
            spec: summary,
            prompt_tasks,
        });
    }
    anyhow::bail!(
        "no eval spec found with id {id:?} under {}",
        specs_dir.display()
    );
}

#[derive(Debug, Serialize)]
struct SpecDetailResponse {
    schema_version: &'static str,
    spec: SpecSummary,
    prompt_tasks: Vec<SpecTaskDetail>,
}

#[derive(Debug, Serialize)]
struct SpecTaskDetail {
    task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    class: Option<String>,
    prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_content: Option<String>,
    expectation_kind: String,
    expectation_value: Value,
}

/// Every `prompt_benchmark` task's declared definition, empty for any other
/// runner kind (or a definition-only spec) — the task table's other kinds
/// (`daedalus_trials`/`cerberus_receipt_bundles`/`harbor_task`) have no
/// per-task prompt/expectation to show, just the task ids `/api/specs`
/// already exposes via `task_ids`.
fn spec_task_details(spec_path: &Path, spec: &EvalSpec) -> Vec<SpecTaskDetail> {
    let Some(runner) = spec.runner.as_ref() else {
        return Vec::new();
    };
    let CorpusSpec::PromptBenchmark { tasks, .. } = &runner.corpus else {
        return Vec::new();
    };
    tasks
        .iter()
        .map(|task| {
            let context_content = task.context_file.as_deref().map(|context_file| {
                let resolved = spec_run::resolve_spec_path_with_alias(spec_path, context_file).path;
                std::fs::read_to_string(&resolved).unwrap_or_else(|err| {
                    format!(
                        "<failed to read context file {}: {err}>",
                        resolved.display()
                    )
                })
            });
            let (expectation_kind, expectation_value) =
                expectation_kind_and_value(&task.expectation);
            SpecTaskDetail {
                task_id: task.task_id.clone(),
                class: task.class.clone(),
                prompt: task.prompt.clone(),
                context_file: task.context_file.clone(),
                context_content,
                expectation_kind,
                expectation_value,
            }
        })
        .collect()
}

fn expectation_kind_and_value(expectation: &crucible_core::PromptExpectation) -> (String, Value) {
    use crucible_core::PromptExpectation::*;
    match expectation {
        Exact { value } => ("exact".to_string(), json!(value)),
        Contains { value } => ("contains".to_string(), json!(value)),
        CaseInsensitiveContains { value } => {
            ("case_insensitive_contains".to_string(), json!(value))
        }
        Regex { pattern } => ("regex".to_string(), json!(pattern)),
        StrictJson { value } => ("strict_json".to_string(), value.clone()),
        PythonUnitTest {
            test_source,
            timeout_ms,
        } => (
            "python_unit_test".to_string(),
            json!({ "test_source": test_source, "timeout_ms": timeout_ms }),
        ),
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
            Some(CorpusSpec::HarborTasks { .. }) => {
                "Deterministic scorer key: Harbor's own verifier reward, parsed from the trial result."
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
        CorpusSpec::HarborTasks { config, tasks } => format!(
            "Runs {} Harbor task{} in a local Docker container via agent {}.",
            tasks.len(),
            plural(tasks.len()),
            config.agent
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
        CorpusSpec::HarborTasks { tasks, .. } => Some(tasks.len()),
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
        CorpusSpec::HarborTasks { tasks, .. } => {
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
        CorpusSpec::HarborTasks { config, tasks } => {
            format!("harbor_tasks agent={} tasks={}", config.agent, tasks.len())
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
    harness: Option<String>,
    since: Option<String>,
    until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<i64>,
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
        harness: nonempty_query(query, "harness"),
        since: nonempty_query(query, "since"),
        until: nonempty_query(query, "until"),
        limit: parse_i64_query(query, "limit")?,
        offset: parse_i64_query(query, "offset")?,
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
            harness: filters.harness.as_deref(),
            since_unix_ms,
            until_unix_ms,
            limit: filters.limit,
            offset: filters.offset,
        },
    )?;
    // Trendlines are derived from exactly this response's rows: an unpaged
    // request (the default, `limit`/`offset` both absent) still draws every
    // point, unchanged from before pagination existed; an explicit paged
    // request draws only that page's points, which is the expected
    // pagination contract, not a regression.
    let trendlines = trendlines(&list.runs);
    Ok(RunsResponse {
        schema_version: RUNS_SCHEMA,
        db: list.db,
        filters,
        runs: list.runs,
        trendlines,
    })
}

fn parse_i64_query(query: &HashMap<String, String>, key: &str) -> Result<Option<i64>> {
    nonempty_query(query, key)
        .map(|value| {
            value.parse::<i64>().with_context(|| {
                format!("query parameter {key:?} must be an integer, got {value:?}")
            })
        })
        .transpose()
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

fn is_matrix_request_error(err: &anyhow::Error) -> bool {
    err.to_string().contains("query param")
}

/// `?benchmark=&limit=` — the eval-detail hub's results-matrix centerpiece:
/// every stored run of one eval as a column, every task either run indexed as
/// a row, and each cell carrying enough of that task's own outcome (pass/fail,
/// response text, latency, response model) that a task drill-down never has
/// to make a second round trip for "every run's actual response side by
/// side". `limit` mirrors `/api/runs`' pagination knob (`None` is
/// unconstrained, matching this benchmark's full run history).
fn matrix_query_response(
    db_path: &Path,
    query: &HashMap<String, String>,
) -> Result<EvalMatrixResponse> {
    let benchmark = query
        .get("benchmark")
        .filter(|value| !value.is_empty())
        .context("missing benchmark query param")?;
    let limit = parse_i64_query(query, "limit")?;
    matrix_response(db_path, benchmark, limit)
}

#[derive(Debug, Serialize)]
struct EvalMatrixResponse {
    schema_version: &'static str,
    benchmark: String,
    columns: Vec<MatrixColumn>,
    rows: Vec<MatrixRow>,
    class_breakdowns: Vec<MatrixClassBreakdown>,
}

#[derive(Debug, Serialize)]
struct MatrixColumn {
    run_id: String,
    config_id: String,
    label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    harness: Option<String>,
    created_at_unix_ms: i64,
    trusted: bool,
    point: Option<f64>,
    lower: f64,
    upper: f64,
    confidence: f64,
    successes: u64,
    n: u64,
}

#[derive(Debug, Serialize)]
struct MatrixRow {
    task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    class: Option<String>,
    cells: Vec<MatrixCell>,
}

#[derive(Debug, Serialize)]
struct MatrixCell {
    run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tracked_results: Vec<run_store::StoredTrackedCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_model: Option<String>,
}

#[derive(Debug, Serialize)]
struct MatrixClassBreakdown {
    class: String,
    columns: Vec<MatrixClassColumn>,
}

#[derive(Debug, Serialize)]
struct MatrixClassColumn {
    run_id: String,
    successes: u64,
    n: u64,
    point: Option<f64>,
}

/// A run's short column label: its model when it declared one (a
/// `prompt_benchmark`/`agentic_judge`/`harbor_task` run), else its config id
/// — the "model or config short-form" the eval-detail card calls for.
fn column_label(run: &run_store::StoredRun) -> String {
    run.model.clone().unwrap_or_else(|| run.config_id.clone())
}

fn matrix_response(
    db_path: &Path,
    benchmark: &str,
    limit: Option<i64>,
) -> Result<EvalMatrixResponse> {
    let list = run_store::list_runs(
        db_path,
        run_store::RunListFilter {
            benchmark: Some(benchmark),
            limit,
            ..Default::default()
        },
    )?;

    let mut columns = Vec::with_capacity(list.runs.len());
    let mut task_class: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut cells_by_task: BTreeMap<String, Vec<MatrixCell>> = BTreeMap::new();
    // class -> run_id -> (successes, n), ordered for a stable response.
    let mut class_totals: BTreeMap<String, BTreeMap<String, (u64, u64)>> = BTreeMap::new();

    for run in &list.runs {
        columns.push(MatrixColumn {
            run_id: run.run_id.clone(),
            config_id: run.config_id.clone(),
            label: column_label(run),
            model: run.model.clone(),
            harness: run.harness.clone(),
            created_at_unix_ms: run.created_at_unix_ms,
            trusted: run.trusted,
            point: run.point,
            lower: run.lower,
            upper: run.upper,
            confidence: run.confidence,
            successes: run.successes,
            n: run.n,
        });

        let detail = run_store::show_run(db_path, &run.run_id)?;
        for task in &detail.prompt_tasks {
            task_class
                .entry(task.task_id.clone())
                .or_insert_with(|| task.class.clone());
            cells_by_task
                .entry(task.task_id.clone())
                .or_default()
                .push(MatrixCell {
                    run_id: run.run_id.clone(),
                    passed: Some(task.passed),
                    tracked_results: task.tracked_results.clone(),
                    output_text: task.output_text.clone(),
                    latency_ms: task.latency_ms,
                    response_model: task.response_model.clone(),
                });
            if let Some(class) = &task.class {
                let entry = class_totals
                    .entry(class.clone())
                    .or_default()
                    .entry(run.run_id.clone())
                    .or_insert((0, 0));
                entry.1 += 1;
                if task.passed {
                    entry.0 += 1;
                }
            }
        }
        for task in &detail.harbor_tasks {
            task_class.entry(task.task_id.clone()).or_insert(None);
            cells_by_task
                .entry(task.task_id.clone())
                .or_default()
                .push(MatrixCell {
                    run_id: run.run_id.clone(),
                    passed: Some(task.passed),
                    tracked_results: Vec::new(),
                    output_text: None,
                    latency_ms: task.latency_ms,
                    response_model: None,
                });
        }
    }

    let rows = task_class
        .into_iter()
        .map(|(task_id, class)| {
            let cells = cells_by_task.remove(&task_id).unwrap_or_default();
            MatrixRow {
                task_id,
                class,
                cells,
            }
        })
        .collect();

    let class_breakdowns = class_totals
        .into_iter()
        .map(|(class, per_run)| MatrixClassBreakdown {
            class,
            columns: per_run
                .into_iter()
                .map(|(run_id, (successes, n))| MatrixClassColumn {
                    run_id,
                    successes,
                    n,
                    point: if n > 0 {
                        Some(successes as f64 / n as f64)
                    } else {
                        None
                    },
                })
                .collect(),
        })
        .collect();

    Ok(EvalMatrixResponse {
        schema_version: MATRIX_SCHEMA,
        benchmark: benchmark.to_string(),
        columns,
        rows,
        class_breakdowns,
    })
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
    /// The same defensible-findings computation the CLI's `--findings-out`
    /// and the MCP `crucible_runs_compare` tool's `include_findings` perform.
    /// `findings` is empty unless `comparison.paired` clears the noise floor.
    findings_journal: crate::findings_journal::FindingsJournal,
}

/// Build the findings journal for one comparison, reusing the same repro
/// command shape the CLI and MCP tool report.
fn findings_journal_for(
    db_path: &Path,
    benchmark: &str,
    left: &str,
    right: &str,
    alpha: f64,
    comparison: &run_store::ConfigComparison,
) -> crate::findings_journal::FindingsJournal {
    let repro_command = crate::runs_compare_repro_command(db_path, benchmark, left, right, alpha);
    crate::findings_journal::journal_from_comparison(comparison, alpha, repro_command)
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
    let out_dir = resolve_requested_run_out(out, spec_path)?;
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
    let base_out = resolve_requested_run_out(out, spec_path)?;
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
        match run_store::compare_configs(db_path, benchmark, left, right, alpha, false) {
            Ok(comparison) => {
                let findings_journal =
                    findings_journal_for(db_path, benchmark, left, right, alpha, &comparison);
                Some(RunComparisonResponse {
                    schema_version: RUN_COMPARISON_SCHEMA,
                    control_label: control_label(&changed_variables),
                    verdict_explanation: verdict_explanation(&comparison),
                    changed_variables,
                    comparison,
                    findings_journal,
                })
            }
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

fn resolve_requested_run_out(out: Option<String>, spec_path: &Path) -> Result<PathBuf> {
    match out {
        Some(requested) => confine_requested_run_out(&requested),
        None => Ok(default_run_out(spec_path)),
    }
}

fn confine_requested_run_out(requested: &str) -> Result<PathBuf> {
    let requested_path = PathBuf::from(requested);
    let cwd = lexical_normalize(&std::env::current_dir().context("reading current directory")?);
    let requested_abs = lexical_normalize(&if requested_path.is_absolute() {
        requested_path.clone()
    } else {
        cwd.join(&requested_path)
    });
    let runs_abs = lexical_normalize(&cwd.join("runs"));
    if !requested_abs.starts_with(&runs_abs) {
        anyhow::bail!(
            "run output path must stay under gitignored runs/; got {}",
            requested_path.display()
        );
    }
    Ok(requested_path)
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

/// Find this run's judgment-queue artifact: the un-panel-scoped `queue.json`
/// a run writes directly (e.g. `recoverable-adjudication-queue`'s), falling
/// back to any `queue.json` (e.g. one copied alongside a pre-rendered static
/// panel) if that is all a run has.
fn find_queue_artifact(detail: &run_store::RunDetail) -> Option<&run_store::StoredArtifact> {
    detail
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
        })
}

/// Where a run's applied labels live: sibling to its `queue.json`, matching
/// `adjudication-panel --serve`'s own default (`<out>/labels.json`) so the
/// same file is readable by `crucible adjudicate --apply` either way.
fn labels_path_for_queue(queue_path: &Path) -> PathBuf {
    queue_path.with_file_name("labels.json")
}

/// `GET /adjudication/panel/<run_id>` — mounts the same live writeback loop
/// [`crate::adjudication_server`]'s standalone `--serve` process runs
/// (crucible-031): when the run has a real `queue.json` artifact, this
/// renders the live-wired panel (verdict taps `POST` to this run's own
/// `.../label` route) with any already-applied labels folded in, instead of
/// the old read-only static projection. Runs that only carry a pre-rendered
/// static `panel/index.html` (no `queue.json` of their own) still fall back
/// to serving that file verbatim — there is no queue model to make live.
fn serve_adjudication_panel(path: &str, db_path: &Path) -> Result<HttpResponse> {
    let raw = path.trim_start_matches("/adjudication/panel/");
    let run_id = percent_decode(raw)?;
    let detail = run_store::show_run(db_path, &run_id)?;
    if let Some(queue_artifact) = find_queue_artifact(&detail) {
        let queue_path = PathBuf::from(&queue_artifact.path);
        let mut queue = load_queue(&queue_path)?;
        let labels_path = labels_path_for_queue(&queue_path);
        queue.labels = adjudication_server::load_existing_labels(&labels_path)?;
        let endpoint = adjudication_label_url(&run_id);
        return Ok(HttpResponse::html(adjudication_panel::render_live_at(
            &queue, &endpoint,
        )));
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

/// `POST /adjudication/panel/<run_id>/label` — the mounted writeback route.
/// Mints and persists a label through the exact same
/// [`adjudication_server::handle_label_post`] the standalone
/// `adjudication-panel --serve` process calls: no forked mint/persist logic,
/// just a stateless per-request load-mutate-persist over the same
/// `labels.json` sibling file (`crucible serve`'s request loop keeps no
/// in-memory session between connections, unlike the standalone server).
fn submit_adjudication_label(path: &str, db_path: &Path, body: &[u8]) -> Result<HttpResponse> {
    let run_id = adjudication_label_run_id(path)?;
    let detail = run_store::show_run(db_path, &run_id)?;
    let queue_artifact = find_queue_artifact(&detail).with_context(|| {
        format!("run {run_id:?} has no adjudication queue artifact to label against")
    })?;
    let queue_path = PathBuf::from(&queue_artifact.path);
    let queue = load_queue(&queue_path)?;
    let labels_path = labels_path_for_queue(&queue_path);
    let mut labels = adjudication_server::load_existing_labels(&labels_path)?;
    let response_body =
        adjudication_server::handle_label_post(body, &queue, &mut labels, &labels_path)?;
    Ok(HttpResponse::new(200, "application/json", response_body))
}

/// Classifies the client-caused failures `submit_adjudication_label` can
/// return as 400s, the same treatment `/api/run`'s `is_run_request_error`
/// gives its own request-shaped errors — anything else (e.g. a DB I/O
/// failure) still falls through to `route()`'s generic 500.
fn is_label_request_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("run id") && message.contains("not found")
        || message.contains("has no adjudication queue artifact")
        || message.contains("is not an adjudication item in this queue")
        || message.contains("parsing label request body as JSON")
        || message.contains("invalid adjudication label path")
}

fn adjudication_label_run_id(path: &str) -> Result<String> {
    let raw = path
        .strip_prefix("/adjudication/panel/")
        .and_then(|rest| rest.strip_suffix("/label"))
        .with_context(|| format!("invalid adjudication label path {path:?}"))?;
    percent_decode(raw)
}

fn artifact_url(run_id: &str, index: usize) -> String {
    format!("/artifacts/{}/{}", percent_encode(run_id), index)
}

fn adjudication_panel_url(run_id: &str) -> String {
    format!("/adjudication/panel/{}", percent_encode(run_id))
}

fn adjudication_label_url(run_id: &str) -> String {
    format!("/adjudication/panel/{}/label", percent_encode(run_id))
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
  <title>Crucible eval arena</title>
  <link rel="icon" type="image/svg+xml" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%231a1a1a' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2'/%3E%3Cpath d='M6.453 15h11.094'/%3E%3Cpath d='M8.5 2h7'/%3E%3C/svg%3E">
  <link rel="stylesheet" href="/assets/aesthetic.css">
  <style>
    /* Crucible identity within the house kit: brass — the crucible's metal.
       Verdict washes derive from the kit's own ok/err tokens so both themes
       inherit correct contrast for free. */
    :root { --ae-accent: #8f6b33; --ae-accent-dark: #c7a366; --cru-ok-wash: color-mix(in srgb, var(--ae-ok) 12%, transparent); --cru-err-wash: color-mix(in srgb, var(--ae-err) 12%, transparent); }
    .cru-desk { display: grid; grid-template-columns: minmax(0, 1fr); gap: var(--ae-space-5); align-content: start; }
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
    .cru-button { appearance: none; border: 1px solid var(--ae-ink); background: var(--ae-ink); color: var(--ae-surface); padding: .65em 1em; min-height: 44px; border-radius: 0; cursor: pointer; transition: transform var(--ae-quick) var(--ae-ease), background var(--ae-quick) var(--ae-ease); }
    .cru-button:hover { transform: translateY(-1px); }
    .cru-button:active { transform: translateY(0); }
    .cru-button.secondary { background: transparent; color: var(--ae-ink); border-color: var(--ae-line); }
    .cru-button:disabled { opacity: .45; cursor: default; transform: none; }
    .cru-icon-button { min-width: 44px; display: inline-flex; align-items: center; justify-content: center; padding: .55em; }
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
    .cru-status.progress .glyph { color: var(--ae-ink-muted); }
    .cru-status.err .glyph { color: var(--ae-err); }
    .cru-status .glyph svg.ae-icon { vertical-align: -0.25em; }
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
    .cru-progress-runner-label { display: none; }
    /* Evals table (nav-level list) and the eval-detail hub it opens into. */
    .cru-back { background: none; border: 0; color: var(--ae-ink-muted); cursor: pointer; padding: 0; font-family: var(--ae-font-mono); font-size: 13px; text-decoration: underline; }
    .cru-hub-head { display: grid; gap: var(--ae-space-2); }
    .cru-hub-meta { display: flex; gap: .6em; flex-wrap: wrap; align-items: center; }
    .cru-section { display: grid; gap: var(--ae-space-3); margin-top: var(--ae-space-5); }
    .cru-section-title { font-weight: var(--ae-w-black); }
    /* Results matrix: first column (task id) pinned while the run columns
       scroll horizontally -- the centerpiece table can have many run
       columns, and losing the task label while scrolling right defeats the
       point of a matrix. */
    .cru-matrix { border-collapse: collapse; width: 100%; }
    .cru-matrix th, .cru-matrix td { border: 1px solid var(--ae-line); padding: .5em .65em; text-align: left; white-space: nowrap; }
    .cru-matrix thead th { background: var(--ae-wash); vertical-align: bottom; }
    .cru-matrix tbody th, .cru-matrix tbody td.task-cell, .cru-matrix tfoot th {
      position: sticky; left: 0; background: var(--ae-surface); z-index: 1;
    }
    .cru-matrix thead th:first-child { position: sticky; left: 0; z-index: 2; }
    .cru-matrix tfoot th { background: var(--ae-wash); }
    .cru-matrix tbody tr:hover td, .cru-matrix tbody tr:hover th { background: var(--ae-wash); }
    .cru-matrix .mark { font-family: var(--ae-font-mono); cursor: pointer; display: inline-block; min-width: 2.1em; text-align: center; padding: .12em .3em; font-weight: var(--ae-w-medium); }
    .cru-matrix .mark.ok { color: var(--ae-ok); background: var(--cru-ok-wash); }
    .cru-matrix .mark.err { color: var(--ae-err); background: var(--cru-err-wash); }
    .cru-matrix .mark.na { color: var(--ae-ink-faint); }
    .cru-matrix tfoot td { font-variant-numeric: tabular-nums; font-weight: var(--ae-w-medium); }
    .cru-matrix tfoot .col-sub { font-weight: var(--ae-w-regular); }
    .cru-matrix .col-label { display: block; font-weight: var(--ae-w-medium); }
    .cru-matrix .col-sub { display: block; color: var(--ae-ink-muted); font-weight: var(--ae-w-regular); font-size: 12px; }
    .cru-drilldown { border: 1px solid var(--ae-ink); background: var(--ae-surface); padding: var(--ae-space-5); display: grid; gap: var(--ae-space-3); }
    .cru-drilldown-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(18em, 1fr)); gap: var(--ae-space-3); }
    .cru-response-card { border: 1px solid var(--ae-line); padding: var(--ae-space-3); display: grid; gap: var(--ae-space-2); align-content: start; }
    .cru-response-card .cru-chip.ok { background: var(--cru-ok-wash); border-color: transparent; }
    .cru-response-card .cru-chip.err { background: var(--cru-err-wash); border-color: transparent; }
    .cru-pre { white-space: pre-wrap; word-break: break-word; font-family: var(--ae-font-mono); font-size: 13px; max-height: 16em; overflow: auto; background: var(--ae-wash); padding: var(--ae-space-2); border-left: 2px solid var(--ae-line); }
    .cru-response-card.ok .cru-pre { border-left-color: var(--ae-ok); }
    .cru-response-card.err .cru-pre { border-left-color: var(--ae-err); }
    .cru-table-wrap .ae-table td, .cru-table-wrap .ae-table th { padding-top: .7em; padding-bottom: .7em; }
    .cru-table-wrap .ae-table td.num { font-variant-numeric: tabular-nums; }
    @media (max-width: 820px) {
      .cru-mobile-bar { display: flex; align-items: center; justify-content: space-between; gap: var(--ae-space-3); padding-bottom: var(--ae-space-4); border-bottom: 1px solid var(--ae-line); }
      .cru-mobile-bar .ae-name { margin: 0; }
      .ae-desk { padding: 1em; }
      .cru-grid, .cru-grid.two, .cru-grid.four, .cru-progress { grid-template-columns: 1fr; }
      .cru-progress > div { border-left: 0; }
      .cru-progress > div:nth-child(-n + 3) { border-top: 1px solid var(--ae-line); }
      .cru-progress > div:first-child { border-top: 0; }
      /* Collapsed to one column, the task/runner grid loses its row/column
         correspondence -- a task name, then each runner's header, then each
         runner's result cell, stack in position order alone. Hide the now-
         redundant header row and instead label each result cell inline with
         its runner, so a viewer never has to count position to tell runners
         apart. */
      .cru-progress-head { display: none; }
      .cru-progress-runner-label { display: inline; font-weight: var(--ae-w-medium); margin-right: .35em; }
    }
    @media (max-width: 480px) {
      /* Phone: the operator's primary surface. Tighten the chrome, keep every
         interactive target >= 44px, and let data tables carry smaller type --
         the sticky task column + per-container horizontal scroll do the rest. */
      .ae-desk { padding: .75em; }
      .cru-desk { gap: var(--ae-space-4); }
      .cru-card, .cru-drilldown { padding: var(--ae-space-4); }
      .cru-matrix th, .cru-matrix td { padding: .45em .5em; font-size: 13px; }
      .cru-table-wrap .ae-table th, .cru-table-wrap .ae-table td { font-size: 13px; }
      .cru-select, .cru-input { min-height: 44px; }
      .cru-toast { left: 1em; right: 1em; max-width: none; }
      .cru-drilldown-grid { grid-template-columns: 1fr; }
      .cru-toolbar { gap: var(--ae-space-2); }
      .cru-hub-meta { font-size: 13px; }
    }
  </style>
</head>
<body>
  <div class="ae-shell">
    <aside class="ae-rail">
      <h1 class="ae-logo"><span class="ae-app-mark"><svg class="ae-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2"/><path d="M6.453 15h11.094"/><path d="M8.5 2h7"/></svg></span><span class="ae-name">CRUCIBLE</span></h1>
      <p class="ae-h">EVAL ARENA</p>
      <nav>
        <button data-view-button="evals" aria-current="page">Evals</button>
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
          <option value="evals">Evals</option>
          <option value="receipts">Receipts</option>
        </select>
      </div>
      <section id="view"></section>
    </main>
  </div>
  <div id="toast" class="cru-toast" hidden></div>
  <script>
    // view is one of 'evals' (the table) | 'eval-detail' (the hub, opened by
    // a row click -- not a persistent nav item) | 'receipts' (global audit
    // trail). NOTE: this markup/CSS is structural scaffolding for the new
    // eval-centric IA (table-first Evals list -> eval-detail hub -> results
    // matrix -> task drill-down); a binding visual design pass follows.
    const state = {
      view: 'evals',
      specs: null,
      runs: null,
      selectedSpecPath: null,
      specDetail: null,
      matrix: null,
      drilldownTaskId: null,
      compareLeft: null,
      compareRight: null,
      compareResult: null,
      selectedRunId: null,
      detail: null,
      activeRun: null,
      filters: {}
    };
    const view = document.querySelector('#view');
    const toast = document.querySelector('#toast');

    function esc(value) {
      return String(value ?? '').replace(/[&<>"']/g, ch => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
    }
    function pct(value) { return value == null ? 'n/a' : (value * 100).toFixed(1) + '%'; }
    function scoreText(run) { return run?.point == null ? 'No score yet' : `${pct(run.point)} (${run.successes}/${run.n} tasks passed)`; }
    function uncertaintyText(run) { return `uncertainty range ${pct(run.lower)} to ${pct(run.upper)}`; }
    // kind is one of 'ok' | 'err' | 'progress' -- 'progress' is a neutral
    // in-progress state (a still-executing run, a receipt not yet detailed),
    // never a judgment, so it rides the same muted ink as supporting text
    // rather than the warn/err hues reserved for actual verdicts.
    const PROGRESS_ICON = '<svg class="ae-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a9 9 0 1 1-6.219-8.56"/></svg>';
    function statusGlyph(kind) {
      const glyph = kind === 'ok' ? '✓' : kind === 'progress' ? PROGRESS_ICON : '×';
      return `<span class="cru-status ${kind}"><span class="glyph">${glyph}</span>`;
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
    function withAuth(options) {
      const next = { ...(options || {}) };
      next.headers = { ...(next.headers || {}) };
      const access = window.sessionStorage.getItem('crucibleServeToken');
      if (access) next.headers.Authorization = 'Bearer ' + access;
      return next;
    }
    async function fetchWithAuth(url, options) {
      let res = await fetch(url, withAuth(options));
      if (res.status === 401) {
        const access = window.prompt('Crucible serve token');
        if (access) {
          window.sessionStorage.setItem('crucibleServeToken', access);
          res = await fetch(url, withAuth(options));
        }
      }
      return res;
    }
    function loadJson(url, options) {
      return fetchWithAuth(url, options).then(async res => {
        const text = await res.text();
        let data;
        try { data = text ? JSON.parse(text) : {}; } catch (_) { data = { error: text }; }
        if (!res.ok) throw new Error(data.error || `${res.status} ${res.statusText}`);
        return data;
      });
    }
    function specs() { return state.specs?.specs || []; }
    function runs() { return state.runs?.runs || []; }
    function specById(id) { return specs().find(spec => spec.id === id); }
    function currentSpec() {
      return specs().find(spec => spec.path === state.selectedSpecPath) || null;
    }
    function lastRunFor(spec) {
      return runs().find(run => run.benchmark_id === spec.id);
    }
    function runsForSpec(spec) {
      return runs().filter(run => run.benchmark_id === spec.id);
    }

    async function refreshAll() {
      const params = new URLSearchParams(state.filters);
      const [specsPayload, runsPayload] = await Promise.all([
        loadJson('/api/specs'),
        loadJson('/api/runs' + (params.toString() ? '?' + params : ''))
      ]);
      state.specs = specsPayload;
      state.runs = runsPayload;
      if (state.selectedRunId) await loadDetail(state.selectedRunId, false).catch(() => {});
      if (state.view === 'eval-detail' && state.selectedSpecPath) await loadEvalDetail(false).catch(() => {});
      render();
    }
    async function loadDetail(runId, rerender = true) {
      state.selectedRunId = runId;
      state.detail = await loadJson('/api/runs/' + encodeURIComponent(runId));
      if (rerender) render();
    }
    // Opens the eval-detail hub for one eval spec (a row click from the
    // Evals table, or an "Open eval" link from Receipts) -- resets every
    // piece of hub-scoped state so a stale matrix/comparison/drill-down from
    // a previously open eval never bleeds into the newly opened one.
    async function openEvalDetail(specPath) {
      state.selectedSpecPath = specPath;
      state.specDetail = null;
      state.matrix = null;
      state.drilldownTaskId = null;
      state.compareResult = null;
      state.compareLeft = null;
      state.compareRight = null;
      state.activeRun = null;
      setView('eval-detail');
      await loadEvalDetail().catch(err => showToast('Eval detail load failed: ' + err.message));
    }
    // Loads the two eval-detail data sources in parallel: the declared task
    // definitions (GET /api/spec) and the results-matrix aggregation
    // (GET /api/matrix). Either can fail independently (e.g. no bearer token
    // yet for the protected matrix route) without blocking the other.
    async function loadEvalDetail(rerender = true) {
      const spec = currentSpec();
      if (!spec) return;
      const [specDetail, matrix] = await Promise.all([
        loadJson('/api/spec?id=' + encodeURIComponent(spec.id)).catch(err => ({ error: err.message })),
        loadJson('/api/matrix?benchmark=' + encodeURIComponent(spec.id)).catch(err => ({ error: err.message }))
      ]);
      state.specDetail = specDetail;
      state.matrix = matrix;
      if (rerender) render();
    }
    function setView(next) {
      state.view = next;
      document.querySelectorAll('[data-view-button]').forEach(button => {
        // The eval-detail hub has no nav button of its own (it opens from a
        // row click, per the redesign's IA); keep "Evals" highlighted while
        // it's open rather than showing no active nav item at all.
        const active = button.dataset.viewButton === next
          || (next === 'eval-detail' && button.dataset.viewButton === 'evals');
        if (active) button.setAttribute('aria-current', 'page');
        else button.removeAttribute('aria-current');
      });
      document.querySelector('#mobile-view').value = next === 'eval-detail' ? 'evals' : next;
      render();
    }
    function render() {
      if (state.view === 'evals') renderEvals();
      if (state.view === 'eval-detail') renderEvalDetail();
      if (state.view === 'receipts') renderReceipts();
    }

    function renderEvals() {
      const rows = specs();
      view.innerHTML = `<div class="cru-toolbar">
        <div><p class="cru-title">Evals</p><p class="cru-lede">An eval is a declared task set with a grader. Pick one to see its results matrix, comparisons, and receipts.</p></div>
        <button class="cru-button secondary cru-icon-button" id="reload" type="button" aria-label="Refresh" title="Refresh"><svg class="ae-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a9 9 0 1 1-2.64-6.36"/><path d="M21 3v6h-6"/></svg></button>
      </div>
      <div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>eval</th><th>runner kind</th><th>tasks</th><th>runs</th><th>last score</th></tr></thead><tbody>
        ${rows.map(spec => {
          const last = lastRunFor(spec);
          const runCount = runsForSpec(spec).length;
          const valid = spec.validation?.valid && spec.validation?.runnable;
          return `<tr class="cru-click" data-eval-row="${esc(spec.path)}">
            <td class="ae-item">${esc(spec.id)}<br><span class="cru-subtle">${esc(spec.benchmark_title)}</span> <span class="cru-chip ${valid ? 'ok' : 'err'}">${valid ? 'ready' : 'needs work'}</span></td>
            <td class="cru-code">${esc(spec.runner_kind || 'none')}</td>
            <td>${esc(spec.task_count_label)}</td>
            <td>${runCount}</td>
            <td>${last ? `${esc(scoreText(last))}<br><span class="cru-subtle">${esc(uncertaintyText(last))}</span>` : '<span class="cru-subtle">No runs yet</span>'}</td>
          </tr>`;
        }).join('')}
      </tbody></table></div>
      ${state.specs?.load_errors?.length ? `<div class="cru-empty">${state.specs.load_errors.map(err => esc(err.path + ': ' + err.error)).join('<br>')}</div>` : ''}`;
      document.querySelector('#reload').onclick = refreshAll;
      document.querySelectorAll('[data-eval-row]').forEach(row => row.onclick = () => openEvalDetail(row.dataset.evalRow));
    }

    // The eval-detail hub: header, results matrix (centerpiece), the task
    // drill-down (when a matrix row is selected), per-class breakdown,
    // pairwise comparisons, the run-launch form + live status, and this
    // eval's own runs -- everything the redesign asked to reach from one
    // eval's row, with no separate Run setup/Live run/Comparison nav items.
    function renderEvalDetail() {
      const spec = currentSpec();
      if (!spec) {
        view.innerHTML = '<div class="cru-empty">Eval not found. <button class="cru-back" id="back-to-evals-missing" type="button">Back to Evals</button></div>';
        document.querySelector('#back-to-evals-missing').onclick = () => setView('evals');
        return;
      }
      const matrix = state.matrix && !state.matrix.error ? state.matrix : null;
      const valid = spec.validation?.valid && spec.validation?.runnable;
      view.innerHTML = `
        <button class="cru-back" id="back-to-evals" type="button">&larr; All evals</button>
        <div class="cru-hub-head">
          <p class="cru-title">${esc(spec.id)}</p>
          <p class="cru-lede">${esc(spec.plain_summary)}</p>
          <p><strong>Decision this informs:</strong> ${esc(spec.decision || 'not declared')}</p>
          <div class="cru-hub-meta">
            <span class="cru-chip ${valid ? 'ok' : 'err'}">${valid ? 'ready' : 'needs work'}</span>
            <span class="cru-chip">${esc(spec.runner_kind || 'no runner')}</span>
            ${(spec.graders || []).map(grader => `<span class="cru-chip">${esc(grader.kind)}:${esc(grader.id)}</span>`).join('')}
          </div>
        </div>

        <section class="cru-section">
          <p class="cru-section-title">Results matrix</p>
          ${renderMatrix(matrix)}
        </section>

        ${state.drilldownTaskId ? `<section class="cru-section">${renderDrilldown(matrix)}</section>` : ''}

        <section class="cru-section">
          <p class="cru-section-title">Per-class breakdown</p>
          ${renderClassBreakdown(matrix)}
        </section>

        <section class="cru-section">
          <p class="cru-section-title">Comparisons</p>
          ${renderComparisons(matrix)}
        </section>

        <section class="cru-section">
          <p class="cru-section-title">Run this eval</p>
          ${renderLaunchForm(spec)}
          <div id="live-status">${renderLive()}</div>
        </section>

        <section class="cru-section">
          <p class="cru-section-title">This eval's runs</p>
          ${renderEvalRuns(spec)}
        </section>
      `;
      wireEvalDetail(spec, matrix);
    }

    function renderMatrix(matrix) {
      if (!matrix) {
        return '<div class="ae-empty"><p class="ae-item">Results matrix unavailable</p><p class="ae-dim">Sign in with the serve token to load stored runs, or launch this eval below.</p></div>';
      }
      if (!matrix.columns.length) {
        return '<div class="ae-empty"><p class="ae-item">No stored runs yet</p><p class="ae-dim">Launch this eval below to populate the results matrix.</p></div>';
      }
      const columns = matrix.columns;
      return `<div class="cru-table-wrap"><table class="cru-matrix">
        <thead><tr><th>task</th>${columns.map(col => `<th><span class="col-label">${esc(col.label)}</span><span class="col-sub">${esc(col.config_id)}</span></th>`).join('')}</tr></thead>
        <tbody>${matrix.rows.map(row => {
          const cellsByRun = new Map(row.cells.map(cell => [cell.run_id, cell]));
          return `<tr class="cru-click" data-task-row="${esc(row.task_id)}">
            <th class="task-cell">${esc(row.task_id)}${row.class ? `<span class="col-sub">${esc(row.class)}</span>` : ''}</th>
            ${columns.map(col => {
              const cell = cellsByRun.get(col.run_id);
              if (!cell || cell.passed == null) return '<td><span class="mark na">n/a</span></td>';
              return `<td><span class="mark ${cell.passed ? 'ok' : 'err'}">${cell.passed ? 'pass' : 'fail'}</span></td>`;
            }).join('')}
          </tr>`;
        }).join('')}</tbody>
        <tfoot><tr><th>score (95% CI)</th>${columns.map(col => `<td class="cru-code">${esc(scoreText(col))}<br><span class="cru-subtle">${esc(uncertaintyText(col))}</span></td>`).join('')}</tr></tfoot>
      </table></div>`;
    }

    function renderClassBreakdown(matrix) {
      if (!matrix || !matrix.class_breakdowns.length) {
        return '<p class="cru-subtle">No class-stratified tasks in this eval.</p>';
      }
      const columns = matrix.columns;
      return `<div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>class</th>${columns.map(col => `<th>${esc(col.label)}</th>`).join('')}</tr></thead><tbody>
        ${matrix.class_breakdowns.map(breakdown => {
          const byRun = new Map(breakdown.columns.map(entry => [entry.run_id, entry]));
          return `<tr><td class="ae-item">${esc(breakdown.class)}</td>${columns.map(col => {
            const entry = byRun.get(col.run_id);
            return `<td>${entry ? `${esc(pct(entry.point))} (${entry.successes}/${entry.n})` : 'n/a'}</td>`;
          }).join('')}</tr>`;
        }).join('')}
      </tbody></table></div>`;
    }

    function renderComparisons(matrix) {
      const columns = matrix?.columns || [];
      if (columns.length < 2) {
        return '<p class="cru-subtle">Store at least two runs of this eval to compare them.</p>';
      }
      const options = columns.map(col => `<option value="${esc(col.config_id)}">${esc(col.label)}</option>`).join('');
      return `<div class="cru-card">
        <p class="cru-subtle">Pick two of this eval's stored runs. Crucible reports the paired verdict and noise-floor diagnosis -- never a bare percentage delta.</p>
        <div class="cru-grid two">
          <label class="cru-field"><span class="cru-label">left</span><select class="cru-select" id="compare-left">${options}</select></label>
          <label class="cru-field"><span class="cru-label">right</span><select class="cru-select" id="compare-right">${options}</select></label>
        </div>
        <div class="cru-actions"><button class="cru-button" id="run-compare" type="button">Compare</button></div>
        <div id="compare-result">${state.compareResult ? renderComparisonPanel(state.compareResult) : ''}</div>
      </div>`;
    }

    // `/api/compare`'s CompareResponse carries the comparison + findings
    // journal but not the launch-time RunComparisonResponse's precomputed
    // control_label/verdict_explanation (those only exist right after a
    // fresh POST /api/run) -- verdictExplanation rebuilds the same plain-
    // English verdict text client-side from `paired`/`common_tasks` so a
    // comparison picked here reads identically to one just launched.
    function verdictExplanation(c) {
      if (c.paired) {
        const interval = `${c.common_tasks} shared task${c.common_tasks === 1 ? '' : 's'}`;
        return c.paired.verdict === 'signal'
          ? `The paired tasks clear the noise floor over ${interval}; this is evidence of a real difference for this eval.`
          : `The shared tasks do not clear the noise floor over ${interval}; with this sample size, treat the result as inconclusive and run more tasks.`;
      }
      return 'Crucible could not pair shared task rows, so this is only a latest-run score difference and not a significance claim.';
    }
    function renderComparisonPanel(result) {
      const c = result.comparison;
      return `<div class="cru-grid two">
        ${scoreCard('left', c.left)}
        ${scoreCard('right', c.right)}
      </div>
      <section class="cru-card" style="margin-top: var(--ae-space-3)">
        <p class="cru-title">Noise floor verdict</p>
        <p>${esc(verdictExplanation(c))}</p>
        <p class="cru-subtle">Plain English: the uncertainty range shows where the true pass rate could plausibly land for this task sample. If the paired result is inside the noise floor, the measured difference is not strong enough to trust yet.</p>
        <pre class="cru-json cru-code">${esc(JSON.stringify(c.paired || { comparison_kind: c.comparison_kind, note: c.note }, null, 2))}</pre>
      </section>
      ${renderFindings(result.findings_journal)}`;
    }

    // Task drill-down (redesign card item 3): the declared definition (from
    // /api/spec's prompt_tasks -- prompt text, resolved context content,
    // expectation) alongside every run's actual response and verdict, read
    // straight out of the already-loaded matrix cells so opening a task
    // needs no further network round trip.
    function renderDrilldown(matrix) {
      const taskId = state.drilldownTaskId;
      const row = matrix?.rows.find(candidate => candidate.task_id === taskId);
      const taskDef = state.specDetail?.prompt_tasks?.find(candidate => candidate.task_id === taskId);
      const columnsById = new Map((matrix?.columns || []).map(col => [col.run_id, col]));
      return `<div class="cru-drilldown">
        <div class="cru-toolbar"><p class="cru-title">Task: ${esc(taskId)}</p><button class="cru-back" id="close-drilldown" type="button">close</button></div>
        ${taskDef ? `
          <p><strong>Prompt</strong></p><pre class="cru-pre">${esc(taskDef.prompt)}</pre>
          ${taskDef.context_content ? `<p><strong>Context file: ${esc(taskDef.context_file)}</strong></p><pre class="cru-pre">${esc(taskDef.context_content)}</pre>` : ''}
          <p><strong>Expectation:</strong> ${esc(taskDef.expectation_kind)} <span class="cru-code">${esc(JSON.stringify(taskDef.expectation_value))}</span></p>
        ` : '<p class="cru-subtle">No declared task definition available for this runner kind.</p>'}
        <p class="cru-section-title">Every run's response</p>
        <div class="cru-drilldown-grid">
          ${(row?.cells || []).map(cell => {
            const column = columnsById.get(cell.run_id);
            return `<div class="cru-response-card ${cell.passed == null ? '' : cell.passed ? 'ok' : 'err'}">
              <p class="cru-title">${esc(column?.label || cell.run_id)}</p>
              <p>${statusGlyph(cell.passed == null ? 'progress' : cell.passed ? 'ok' : 'err')}${cell.passed == null ? 'no data' : cell.passed ? 'pass' : 'fail'}</span></p>
              ${cell.output_text != null ? `<pre class="cru-pre">${esc(cell.output_text)}</pre>` : '<p class="cru-subtle">No response text indexed.</p>'}
              <p class="cru-subtle">${cell.latency_ms != null ? esc(cell.latency_ms + 'ms') : ''} ${esc(cell.response_model || '')}</p>
            </div>`;
          }).join('')}
        </div>
      </div>`;
    }

    function renderEvalRuns(spec) {
      const rows = runsForSpec(spec);
      if (!rows.length) return '<p class="cru-subtle">No stored runs for this eval yet.</p>';
      return `<div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>score</th><th>runner</th><th>time</th><th></th></tr></thead><tbody>
        ${rows.map(run => `<tr><td>${esc(scoreText(run))}<br><span class="cru-subtle">${esc(uncertaintyText(run))}</span></td><td class="wrap cru-code">${esc(run.config_id)}<br>${esc(run.model || run.provider || 'deterministic')}</td><td>${esc(new Date(run.created_at_unix_ms).toISOString())}</td><td><button class="cru-button secondary" data-open-receipt="${esc(run.run_id)}" type="button">Open receipt</button></td></tr>`).join('')}
      </tbody></table></div>`;
    }

    function wireEvalDetail(spec, matrix) {
      document.querySelector('#back-to-evals').onclick = () => setView('evals');
      document.querySelectorAll('[data-task-row]').forEach(row => row.onclick = () => {
        state.drilldownTaskId = row.dataset.taskRow;
        renderEvalDetail();
      });
      document.querySelector('#close-drilldown')?.addEventListener('click', () => {
        state.drilldownTaskId = null;
        renderEvalDetail();
      });
      const leftSelect = document.querySelector('#compare-left');
      const rightSelect = document.querySelector('#compare-right');
      if (leftSelect && rightSelect) {
        const columns = matrix?.columns || [];
        leftSelect.value = state.compareLeft || columns[0]?.config_id || '';
        rightSelect.value = state.compareRight || columns[1]?.config_id || '';
        document.querySelector('#run-compare').onclick = async () => {
          state.compareLeft = leftSelect.value;
          state.compareRight = rightSelect.value;
          try {
            const params = new URLSearchParams({ benchmark: spec.id, left: state.compareLeft, right: state.compareRight });
            state.compareResult = await loadJson('/api/compare?' + params);
          } catch (err) {
            showToast('Compare failed: ' + err.message);
          }
          renderEvalDetail();
        };
      }
      document.querySelectorAll('[data-runner-field]').forEach(input => input.oninput = updateDiffBox);
      document.querySelector('#launch').onclick = () => launchComparison(spec);
      updateDiffBox();
      // The empty live-status state's CTA has nowhere standalone to send the
      // operator to anymore (Run setup was folded into this same section,
      // not left as its own view) -- it focuses the launch form already on
      // screen instead of navigating away from it.
      document.querySelector('#live-empty-cta')?.addEventListener('click', () => {
        document.querySelector('[data-runner-field="runner-a"][name="model"]')?.focus();
      });
      document.querySelectorAll('[data-open-receipt]').forEach(button => button.onclick = async () => {
        await loadDetail(button.dataset.openReceipt, false);
        setView('receipts');
      });
    }

    function renderLaunchForm(spec) {
      const defaults = spec.runner_defaults || {};
      return `<section class="cru-card">
        <p>${esc(spec.plain_summary)}</p>
        <p><strong>Locked verifier:</strong> ${esc(spec.verifier_summary)}</p>
        <p><strong>Locked tool policy:</strong> ${esc(defaults.tool_policy || 'No tool policy declared for this runner.')}</p>
      </section>
      <div class="cru-grid two" style="margin-top: var(--ae-space-4)">
        ${runnerEditor('runner-a', 'Runner A', defaults)}
        ${runnerEditor('runner-b', 'Runner B', { ...defaults, model: alternateModel(defaults.model) })}
      </div>
      <section class="cru-card warning" style="margin-top: var(--ae-space-4)" id="diff-box"></section>
      <div class="cru-actions"><button class="cru-button" id="launch" ${spec.supports_controlled_comparison ? '' : 'disabled'} type="button">Launch controlled comparison</button></div>`;
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
    // Launches this eval's controlled comparison inline, scoped to whichever
    // eval-detail hub is currently open (redesign card item 2: "the 'run
    // this eval' launch flow ... after launch, live status shows here"). The
    // live-run polling/progress mechanics (state.activeRun, renderLive,
    // taskCell) are unchanged from the old standalone Live-run view -- only
    // relocated into this hub instead of a separate nav destination.
    async function launchComparison(spec) {
      const runners = [runnerFrom('runner-a'), runnerFrom('runner-b')];
      const tasks = spec.task_ids?.length ? spec.task_ids : Array.from({ length: spec.task_count || 1 }, (_, index) => `task-${index + 1}`);
      state.activeRun = { status: 'running', spec, runners, tasks, startedAt: new Date().toISOString(), response: null, error: null };
      renderEvalDetail();
      try {
        const response = await loadJson('/api/run', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ spec: spec.path, runners })
        });
        state.activeRun = { ...state.activeRun, status: 'complete', response };
        if (response.comparison) state.compareResult = response.comparison;
        showToast(response.comparison ? 'Comparison stored and paired.' : 'Runs stored; comparison needs attention.');
        await refreshAll();
      } catch (err) {
        state.activeRun = { ...state.activeRun, status: 'failed', error: err.message };
        renderEvalDetail();
        showToast('Run failed: ' + err.message);
      }
    }

    // Renders the live-run status panel embedded in the eval-detail hub's
    // "Run this eval" section (see `renderEvalDetail`'s `#live-status` div).
    // Returns markup rather than assigning `view.innerHTML` directly, since
    // it is only ever a fragment of the larger hub render now.
    function renderLive() {
      const active = state.activeRun;
      if (!active) {
        return `<div class="ae-empty">
          <p class="ae-item"><svg class="ae-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"/></svg> No active run</p>
          <p class="ae-dim">Launch a controlled comparison above to watch it execute here.</p>
          <p><button class="cru-button" id="live-empty-cta" type="button">Focus runner settings</button></p>
        </div>`;
      }
      const done = active.status === 'complete';
      const failed = active.status === 'failed';
      return `<section class="cru-card ${failed ? 'warning' : ''}"><p>${statusGlyph(failed ? 'err' : done ? 'ok' : 'progress')}${failed ? 'failed' : done ? 'complete' : 'running'}</span> ${failed ? esc(active.error) : done ? 'Both runner receipts are stored.' : 'Crucible is executing the runner bundle now.'}</p></section>
      <div class="cru-progress" style="margin-top: var(--ae-space-4)">
        <div class="cru-label cru-progress-head">task</div>${active.runners.map(runner => `<div class="cru-label cru-progress-head">${esc(runner.id || runner.model)}</div>`).join('')}
        ${active.tasks.map(task => `<div class="cru-code">${esc(task)}</div>${active.runners.map(runner => taskCell(active, runner, task)).join('')}`).join('')}
      </div>`;
    }
    function taskCell(active, runner, taskId) {
      // The runner label repeats inline in every cell (shown only at the
      // mobile breakpoint via .cru-progress-runner-label) so the task/runner
      // grouping survives the grid collapsing to one column: nothing here
      // depends on counting position to know which runner a result is for.
      const runnerLabel = `<span class="cru-progress-runner-label">${esc(runner.id || runner.model)}</span>`;
      if (!active.response) return `<div>${runnerLabel}${statusGlyph('progress')}running</span></div>`;
      const run = (active.response.runs || []).find(row => row.runner_id === runner.id || row.model === runner.model);
      const detail = run?.report?.evals?.[0];
      if (!detail) return `<div>${runnerLabel}${statusGlyph('progress')}stored</span></div>`;
      return `<div>${runnerLabel}${statusGlyph('ok')}receipt written</span><br><span class="cru-subtle">${esc(scoreText(run))}</span></div>`;
    }

    function renderFindings(journal) {
      const findings = journal?.findings || [];
      if (!findings.length) return '';
      return `<section class="cru-card" style="margin-top: var(--ae-space-4)">
        <p class="cru-title">Defensible findings</p>
        <p class="cru-subtle">Crucible only mints a finding record when the paired result clears the noise floor above — this is the same crucible.findings_journal.v1 artifact crucible runs compare --findings-out writes.</p>
        ${findings.map(finding => `<div class="cru-card" style="margin-top: var(--ae-space-2)">
          <p>${esc(finding.hypothesis)}</p>
          <p class="cru-subtle">Δ ${esc(finding.delta.point.toFixed(4))} [${esc(finding.delta.lower.toFixed(4))}, ${esc(finding.delta.upper.toFixed(4))}] over ${esc(finding.delta.common_tasks)} shared tasks, ${esc((finding.delta.confidence * 100).toFixed(0))}% confidence.</p>
          <p class="cru-code">${esc(finding.repro_command)}</p>
        </div>`).join('')}
      </section>`;
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
      view.innerHTML = `<div class="cru-toolbar"><div><p class="cru-title">Receipts</p><p class="cru-lede">Stored run records and artifacts -- the global audit trail underneath every eval.</p></div></div>
      <div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>eval</th><th>score</th><th>runner</th><th>time</th></tr></thead><tbody>
        ${rows.map(run => `<tr class="cru-click" data-run-id="${esc(run.run_id)}"><td class="ae-item">${esc(run.benchmark_id)}</td><td>${esc(scoreText(run))}<br><span class="cru-subtle">${esc(uncertaintyText(run))}</span></td><td class="wrap cru-code">${esc(run.config_id)}<br>${esc(run.model || run.provider || 'deterministic')}</td><td>${esc(new Date(run.created_at_unix_ms).toISOString())}</td></tr>`).join('')}
      </tbody></table></div>
      ${detail ? renderDetail(detail) : ''}`;
      document.querySelectorAll('[data-run-id]').forEach(row => row.onclick = async () => { await loadDetail(row.dataset.runId, true); });
      document.querySelectorAll('[data-artifact-url]').forEach(link => link.onclick = openArtifact);
      document.querySelector('#detail-open-eval')?.addEventListener('click', () => {
        const spec = specById(detail.run.benchmark_id);
        if (spec) openEvalDetail(spec.path);
      });
    }
    function renderDetail(detail) {
      const run = detail.run;
      const artifactHref = index => `/artifacts/${encodeURIComponent(run.run_id)}/${index}`;
      const spec = specById(run.benchmark_id);
      return `<section class="cru-card" style="margin-top: var(--ae-space-4)">
        <div class="cru-toolbar"><p class="cru-title">Run receipt</p>${spec ? '<button class="cru-button secondary" id="detail-open-eval" type="button">Open eval</button>' : ''}</div>
        <p class="cru-code">${esc(run.run_id)}</p>
        <div class="cru-grid two"><div><p>${esc(scoreText(run))}</p>${ci(run)}<p class="cru-subtle">${esc(uncertaintyText(run))}</p></div><dl class="cru-code"><dt>eval</dt><dd>${esc(run.benchmark_id)}</dd><dt>runner</dt><dd>${esc(run.runner_kind)}</dd><dt>report</dt><dd>${esc(run.run_report)}</dd></dl></div>
        <p class="cru-title">Task results</p>${renderTasks(detail)}
        <p class="cru-title">Artifacts</p>${detail.artifacts.length ? `<table class="ae-table"><tbody>${detail.artifacts.map((artifact, index) => `<tr><td>${esc(artifact.kind)}</td><td class="wrap cru-code">${esc(artifact.path)}</td><td><a href="${artifactHref(index)}" data-artifact-url="${artifactHref(index)}" target="_blank" rel="noreferrer">open</a></td></tr>`).join('')}</tbody></table>` : '<p class="cru-subtle">No artifacts indexed.</p>'}
      </section>`;
    }
    async function openArtifact(event) {
      event.preventDefault();
      try {
        const res = await fetchWithAuth(event.currentTarget.dataset.artifactUrl);
        const blob = await res.blob();
        if (!res.ok) throw new Error(await blob.text());
        const url = URL.createObjectURL(blob);
        window.open(url, '_blank', 'noopener');
        window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
      } catch (err) {
        showToast('Artifact open failed: ' + err.message);
      }
    }
    function renderTasks(detail) {
      if (detail.prompt_tasks.length) {
        return `<div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>task</th><th>verdict</th><th>latency</th><th>model</th><th>cost</th></tr></thead><tbody>${detail.prompt_tasks.map(task => `<tr><td class="ae-item">${esc(task.task_id)}</td><td>${statusGlyph(task.passed ? 'ok' : 'err')}${task.passed ? 'pass' : 'fail'}</span></td><td>${task.latency_ms == null ? 'n/a' : esc(task.latency_ms + 'ms')}</td><td>${esc(task.response_model || task.requested_model || '')}</td><td>${task.cost_usd == null ? 'n/a' : '$' + Number(task.cost_usd).toFixed(5)}</td></tr>`).join('')}</tbody></table></div>`;
      }
      if (detail.task_results && detail.task_results.length) {
        return `<div class="cru-table-wrap"><table class="ae-table"><thead><tr><th>task</th><th>trial</th><th>matched</th><th>missed</th><th>false positives</th><th>verifier</th></tr></thead><tbody>${detail.task_results.map(task => `<tr><td class="ae-item">${esc(task.task_id)}</td><td>${esc(task.trial ?? '')}</td><td>${esc(task.matched ?? '')}/${esc(task.expected_defects ?? '')}</td><td>${esc(task.missed ?? '')}</td><td>${esc(task.false_positives ?? '')}</td><td>${statusGlyph(!task.error && !task.scorer_error ? 'ok' : 'err')}${task.error || task.scorer_error ? esc(task.error || task.scorer_error) : 'graded'}</span></td></tr>`).join('')}</tbody></table></div>`;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn query(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    /// A fresh scratch specs dir under the system temp dir, mirroring
    /// `test_fixtures::temp_db`'s shape for the spec-detail/matrix tests below.
    fn temp_specs_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crucible-serve-specs-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp specs dir");
        dir
    }

    /// `GET /api/spec?id=` is the eval-detail hub's task drill-down source:
    /// it must resolve a `context_file` declared relative to the spec (the
    /// same resolution `spec_run`'s runner performs before a live model
    /// call) and report each task's expectation kind/value alongside the
    /// prompt text.
    #[test]
    fn spec_detail_response_resolves_context_file_and_expectation() {
        let dir = temp_specs_dir("context-file");
        std::fs::write(dir.join("context.txt"), "the long context body").unwrap();
        std::fs::write(
            dir.join("with-context-v0.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": "crucible.eval_spec.v1",
                "id": "with-context-v0",
                "task": "with-context",
                "inputs": "one task with a context file",
                "outputs": "text",
                "graders": { "graders": [{ "id": "contains", "kind": "deterministic" }] },
                "aggregation": "proportion",
                "uncertainty": { "method": "wilson", "confidence": 0.95 },
                "decision": "test",
                "runner": {
                    "kind": "prompt_benchmark",
                    "corpus": {
                        "source": "prompt_benchmark",
                        "config": {
                            "provider": "open_router",
                            "model": "openrouter/auto",
                            "system_prompt": "sys",
                            "credential_env": "OPENROUTER_API_KEY"
                        },
                        "tasks": [{
                            "task_id": "t1",
                            "class": "extraction",
                            "context_file": "context.txt",
                            "prompt": "read the context",
                            "expectation": { "kind": "contains", "value": "needle" }
                        }]
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let response =
            spec_detail_response(&dir, &query(&[("id", "with-context-v0")])).expect("spec found");
        assert_eq!(response.schema_version, SPEC_DETAIL_SCHEMA);
        assert_eq!(response.spec.id, "with-context-v0");
        assert_eq!(response.prompt_tasks.len(), 1);
        let task = &response.prompt_tasks[0];
        assert_eq!(task.task_id, "t1");
        assert_eq!(task.class.as_deref(), Some("extraction"));
        assert_eq!(task.context_file.as_deref(), Some("context.txt"));
        assert_eq!(
            task.context_content.as_deref(),
            Some("the long context body"),
            "context_content must hold the resolved file's content, not just its declared path"
        );
        assert_eq!(task.expectation_kind, "contains");
        assert_eq!(task.expectation_value, json!("needle"));
    }

    #[test]
    fn spec_detail_response_reports_a_classifiable_error_for_an_unknown_id() {
        let dir = temp_specs_dir("unknown-id");
        let err = spec_detail_response(&dir, &query(&[("id", "does-not-exist")]))
            .expect_err("no spec has this id");
        assert!(is_spec_detail_request_error(&err));
    }

    #[test]
    fn spec_detail_response_reports_a_classifiable_error_for_a_missing_id_param() {
        let dir = temp_specs_dir("missing-id");
        let err = spec_detail_response(&dir, &query(&[])).expect_err("id param is required");
        assert!(is_spec_detail_request_error(&err));
    }

    /// `GET /api/matrix` is the results-matrix centerpiece: every stored run
    /// of one eval as a column, every task either run indexed as a row. Seeds
    /// the same 10-shared-task fixture `api_compare_*` uses (two runs,
    /// `t0`..`t9`, a 1-vs-9 discordant split) and checks the matrix reflects
    /// exactly that shape rather than re-deriving pass/fail from the
    /// comparison layer.
    #[test]
    fn matrix_response_aggregates_tasks_as_rows_and_runs_as_columns() {
        let db = crate::test_fixtures::temp_db("serve-matrix-signal");
        crate::test_fixtures::seed_paired_signal(&db);

        let response = matrix_response(&db, crate::test_fixtures::BENCHMARK, None)
            .expect("matrix query succeeds");
        assert_eq!(response.schema_version, MATRIX_SCHEMA);
        assert_eq!(response.columns.len(), 2, "one column per stored run");
        assert_eq!(response.rows.len(), 10, "one row per shared task t0..t9");

        let labels: std::collections::BTreeSet<_> =
            response.columns.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(crate::test_fixtures::LEFT_MODEL));
        assert!(labels.contains(crate::test_fixtures::RIGHT_MODEL));

        let left_column = response
            .columns
            .iter()
            .find(|c| c.label == crate::test_fixtures::LEFT_MODEL)
            .expect("left column present");
        let row_t0 = response
            .rows
            .iter()
            .find(|row| row.task_id == "t0")
            .expect("row t0 present");
        let left_cell_t0 = row_t0
            .cells
            .iter()
            .find(|cell| cell.run_id == left_column.run_id)
            .expect("left cell for t0 present");
        assert_eq!(
            left_cell_t0.passed,
            Some(true),
            "seed_paired_signal makes only i == 0 (t0) pass on the left run"
        );
        assert_eq!(
            left_cell_t0.tracked_results,
            vec![run_store::StoredTrackedCheck {
                id: "style".to_string(),
                passed: false,
            }],
            "tracked outcomes ride alongside the gate verdict instead of replacing it"
        );
        assert_eq!(row_t0.class.as_deref(), Some("format_adherence"));

        assert_eq!(
            response.class_breakdowns.len(),
            1,
            "every seeded task shares the single format_adherence class"
        );
        let breakdown = &response.class_breakdowns[0];
        assert_eq!(breakdown.class, "format_adherence");
        assert_eq!(breakdown.columns.len(), 2);
        for column in &breakdown.columns {
            assert_eq!(column.n, 10);
        }
    }

    #[test]
    fn matrix_response_reports_a_classifiable_error_on_a_missing_benchmark_param() {
        let db = crate::test_fixtures::temp_db("serve-matrix-missing-param");
        let err = matrix_query_response(&db, &query(&[])).expect_err("benchmark param is required");
        assert!(is_matrix_request_error(&err));
    }

    // These unit tests call `compare_query_response` — the same handler
    // `route()` dispatches `GET /api/compare` to — directly rather than
    // through `route()`/`protected()`. `route()`'s bearer-auth gate is a
    // generic wrapper already covered end-to-end for other routes in
    // `tests/cli.rs` via a spawned `crucible serve` subprocess; calling it
    // here would mean mutating the process-global `CRUCIBLE_SERVE_TOKEN` env
    // var from these in-process, parallel unit tests, which is exactly the
    // kind of shared mutable state this repo's tests otherwise avoid. The
    // decision inside that gate is extracted into the pure `authorize`, which
    // these tests exercise directly with no env at all.

    const SELF: Option<&str> = Some("127.0.0.1:4174");

    #[test]
    fn default_mode_allows_non_browser_and_same_origin_callers() {
        // No token, no trust-network = same-origin mode: curl/CLI/agent calls
        // (no Origin header) and this UI's own browser requests pass with zero
        // configuration.
        assert_eq!(authorize(false, None, None, None, SELF), Ok(()));
        assert_eq!(
            authorize(false, None, None, Some("http://127.0.0.1:4174"), SELF),
            Ok(())
        );
        // Scheme and letter case are not identity; authority is.
        assert_eq!(
            authorize(false, None, None, Some("https://127.0.0.1:4174/"), SELF),
            Ok(())
        );
    }

    #[test]
    fn default_mode_refuses_a_foreign_browser_origin() {
        // The drive-by CSRF vector: a web page the operator happens to have
        // open fires fetch() at localhost. Browsers stamp the page's real
        // origin and scripts cannot forge it — refuse anything foreign,
        // including the opaque "null" origin of sandboxed/redirect contexts.
        let deny = Err(Deny::Forbidden(
            "cross-origin request refused; set CRUCIBLE_SERVE_TOKEN for cross-origin API access",
        ));
        assert_eq!(
            authorize(false, None, None, Some("https://evil.example"), SELF),
            deny
        );
        assert_eq!(authorize(false, None, None, Some("null"), SELF), deny);
        // No Host to compare against => nothing can match.
        assert_eq!(
            authorize(false, None, None, Some("http://127.0.0.1:4174"), None),
            deny
        );
    }

    #[test]
    fn authorize_matches_a_configured_bearer_token() {
        let unauthorized = Err(Deny::Unauthorized("authorization bearer token required"));
        assert_eq!(
            authorize(false, Some("s3cret"), Some("Bearer s3cret"), None, SELF),
            Ok(())
        );
        assert_eq!(
            authorize(false, Some("s3cret"), Some("Bearer wrong"), None, SELF),
            unauthorized
        );
        assert_eq!(
            authorize(false, Some("s3cret"), None, None, SELF),
            unauthorized
        );
        // Token mode is the only defense that also covers non-browser local
        // processes; a valid bearer passes regardless of Origin so legitimate
        // cross-origin API consumers keep working.
        assert_eq!(
            authorize(
                false,
                Some("s3cret"),
                Some("Bearer s3cret"),
                Some("https://elsewhere.example"),
                SELF
            ),
            Ok(())
        );
    }

    #[test]
    fn trust_network_opts_out_of_the_gate_entirely() {
        // Behind a trusted front the gate is off regardless of token, header,
        // or origin — a reverse proxy may rewrite Host, so no origin check.
        assert_eq!(authorize(true, None, None, None, None), Ok(()));
        assert_eq!(
            authorize(
                true,
                Some("s3cret"),
                Some("Bearer wrong"),
                Some("https://elsewhere.example"),
                None
            ),
            Ok(())
        );
    }

    /// `GET /api/compare` is the serve face's analog of `crucible runs
    /// compare` and the MCP `crucible_runs_compare` tool: it must expose the
    /// findings journal (non-empty when the paired verdict clears the noise
    /// floor) without any new run being launched.
    #[test]
    fn api_compare_includes_a_findings_journal_for_a_paired_signal() {
        let db = crate::test_fixtures::temp_db("serve-compare-signal");
        crate::test_fixtures::seed_paired_signal(&db);
        let query = query(&[
            ("benchmark", crate::test_fixtures::BENCHMARK),
            ("left", crate::test_fixtures::LEFT_MODEL),
            ("right", crate::test_fixtures::RIGHT_MODEL),
        ]);

        let response = compare_query_response(&db, &query).expect("compare query succeeds");
        assert_eq!(response.schema_version, COMPARE_SCHEMA);
        assert_eq!(response.comparison.comparison_kind, "paired_mcnemar");
        assert_eq!(
            response.findings_journal.findings.len(),
            1,
            "a paired signal must mint exactly one finding record: {:?}",
            response.findings_journal
        );
        assert_eq!(
            response.findings_journal.findings[0].paired.verdict,
            crucible_core::DeltaVerdict::Signal
        );
    }

    /// Parity with the CLI's own non-regression test (`tests/cli.rs`,
    /// `--findings-out` on an unpaired comparison) and the MCP test of the
    /// same scenario: inside the noise floor, `/api/compare` must show zero
    /// finding records too.
    #[test]
    fn api_compare_shows_no_findings_inside_the_noise_floor() {
        let db = crate::test_fixtures::temp_db("serve-compare-noise-floor");
        crate::test_fixtures::seed_paired_inside_noise_floor(&db);
        let query = query(&[
            ("benchmark", crate::test_fixtures::BENCHMARK),
            ("left", crate::test_fixtures::LEFT_MODEL),
            ("right", crate::test_fixtures::RIGHT_MODEL),
        ]);

        let response = compare_query_response(&db, &query).expect("compare query succeeds");
        assert_eq!(response.comparison.comparison_kind, "paired_mcnemar");
        assert_eq!(
            response.findings_journal.findings.len(),
            0,
            "an inside-noise-floor paired comparison must mint no finding records: {:?}",
            response.findings_journal
        );
    }

    #[test]
    fn api_compare_reports_a_classifiable_error_on_a_missing_required_query_param() {
        let db = crate::test_fixtures::temp_db("serve-compare-missing-param");
        crate::test_fixtures::seed_paired_signal(&db);
        let query = query(&[("left", crate::test_fixtures::LEFT_MODEL)]);

        let err =
            compare_query_response(&db, &query).expect_err("benchmark and right are both missing");
        assert!(
            is_compare_request_error(&err),
            "a missing required query param must classify as a client error (400), not a 500: {err}"
        );
    }

    /// Extract the declaration body of the first CSS rule with an exact
    /// selector (e.g. `.cru-desk`) inside `css`, panicking if the selector
    /// is not found. Assumes a single-line-ish `selector { decls }` rule,
    /// which is how `render_index`'s inline `<style>` block is written.
    fn css_rule_body<'a>(css: &'a str, selector: &str) -> &'a str {
        let needle = format!("{selector} {{");
        let start = css
            .find(&needle)
            .unwrap_or_else(|| panic!("selector {selector:?} not found in: {css}"));
        let body_start = start + needle.len();
        let end = css[body_start..]
            .find('}')
            .unwrap_or_else(|| panic!("unterminated rule for {selector:?}"));
        &css[body_start..body_start + end]
    }

    /// crucible-940 bug #1: `.cru-desk` is the grid parent for every desk
    /// view (Receipts, Run-detail, Live-run, ...). Without an explicit
    /// `grid-template-columns`, a single-flex-child toolbar (Receipts,
    /// Run-detail) lets the implicit grid column size to its child's
    /// intrinsic content width, blowing out past the viewport at mobile
    /// widths (measured 1014px content in a 375px viewport). `minmax(0, 1fr)`
    /// lets the sole column shrink below that intrinsic width instead of a
    /// bare `1fr` track, which still respects intrinsic minimums.
    #[test]
    fn cru_desk_grid_has_explicit_shrinkable_column() {
        let html = render_index();
        let body = css_rule_body(&html, ".cru-desk");
        assert!(
            body.contains("grid-template-columns"),
            ".cru-desk must declare grid-template-columns so an implicit \
             single-child column cannot blow out past the viewport: {body}"
        );
        assert!(
            body.contains("minmax(0"),
            ".cru-desk's grid-template-columns must allow the column to \
             shrink below its content's intrinsic width (minmax(0, ...)), \
             not just declare a bare 1fr track: {body}"
        );
    }

    /// crucible-940 bug #2: the Live-run task x runner progress grid must not
    /// lose the row/column correspondence when it collapses to one column at
    /// the mobile breakpoint -- each result cell needs the runner's name
    /// inline so a viewer isn't counting position to tell runners apart.
    #[test]
    fn live_run_progress_cells_carry_runner_label_for_mobile_grouping() {
        let html = render_index();
        assert!(
            html.contains("cru-progress-runner-label"),
            "expected a runner-label hook usable inside each result cell: {html}"
        );
        // taskCell() must render the label into every branch it returns, not
        // just the (position-only) header row above the grid.
        let task_cell_start = html
            .find("function taskCell")
            .expect("taskCell function present");
        let task_cell_end = html[task_cell_start..]
            .find("\n    function ")
            .map(|offset| task_cell_start + offset)
            .unwrap_or(html.len());
        let task_cell_body = &html[task_cell_start..task_cell_end];
        assert!(
            task_cell_body.contains("cru-progress-runner-label"),
            "taskCell() must emit the runner label inline in every branch \
             so mobile users can tell which runner a result belongs to: {task_cell_body}"
        );
    }

    /// Extract the body of a top-level JS function declared as
    /// `function <name>(` inside `render_index`'s inline `<script>`, up to
    /// the next top-level `function` declaration (or end of string).
    fn js_function_body<'a>(html: &'a str, name: &str) -> &'a str {
        let needle = format!("function {name}(");
        let start = html
            .find(&needle)
            .unwrap_or_else(|| panic!("function {name:?} not found in rendered index"));
        let end = html[start..]
            .find("\n    function ")
            .map(|offset| start + offset)
            .unwrap_or(html.len());
        &html[start..end]
    }

    /// crucible-941: the Live-run empty state ("no active run yet") was a
    /// single line of gray text with no next-step affordance -- the weakest
    /// screen in the app next to Comparison/Benchmarks/Setup, which all
    /// carry a heading, supporting line, and (where relevant) a real button.
    /// Guard the fix at the source level: renderLive()'s empty branch must
    /// use the design system's own `.ae-empty` absence recipe (heading +
    /// supporting line + one quiet action), not reinvent a bespoke bare div,
    /// and must wire a real `.cru-button` CTA. Post eval-centric-redesign
    /// (crucible-ui-eval-centric-redesign): Run setup is no longer a
    /// separate nav view -- it is the launch form embedded in this same
    /// eval-detail section -- so the CTA now focuses that on-screen form
    /// (`live-empty-cta`) rather than navigating to a `setup` view that no
    /// longer exists.
    #[test]
    fn live_run_empty_state_has_a_real_cta_and_design_system_weight() {
        let html = render_index();
        let render_live = js_function_body(&html, "renderLive");
        assert!(
            render_live.contains("ae-empty"),
            "the empty Live-run branch must use the Aesthetic .ae-empty \
             absence recipe rather than a bare, unstyled div: {render_live}"
        );
        assert!(
            render_live.contains("cru-button") && render_live.contains(r#"id="live-empty-cta""#),
            "the empty Live-run branch must offer a real, addressable button \
             (not a bare link): {render_live}"
        );
        assert!(
            render_live.contains("ae-item") && render_live.contains("ae-dim"),
            "the empty Live-run branch must carry a heading and a supporting \
             line, matching the design system's own empty-state weight: {render_live}"
        );
        assert!(
            !render_live.contains("cru-empty\">No active run"),
            "the old single-line unstyled empty state must be gone: {render_live}"
        );
    }

    /// crucible-941: while a run is in progress, `statusGlyph` painted the
    /// glyph with the `warn` class (an amber '!') -- indistinguishable at a
    /// glance from a real warning. In-progress is not a judgment, so it must
    /// ride neutral ink (`--ae-ink-muted`) via a dedicated `progress` state,
    /// never the warn/err hues reserved for actual verdicts.
    #[test]
    fn running_status_glyph_is_neutral_not_warning_coded() {
        let html = render_index();
        assert!(
            !html.contains(".cru-status.warn"),
            "no rendered status should still be styled with the warn hue: {html}"
        );
        assert!(
            html.contains(".cru-status.progress .glyph { color: var(--ae-ink-muted); }"),
            "the in-progress glyph must ride neutral ink, not a warning color"
        );
        let status_glyph = js_function_body(&html, "statusGlyph");
        assert!(
            status_glyph.contains("'progress'") && !status_glyph.contains("'warn'"),
            "statusGlyph must expose an explicit neutral 'progress' state, \
             not the removed boolean warn flag: {status_glyph}"
        );
        // Regression guard: none of the "in progress" call sites (the
        // running summary line, and the two still-executing taskCell
        // branches) may regress to the old boolean (ok, warn) signature.
        assert!(
            !html.contains("statusGlyph(false, true)"),
            "a taskCell branch is still using the removed boolean warn signature: {html}"
        );
        assert!(
            html.contains("statusGlyph('progress')"),
            "the in-progress call sites must use the explicit neutral state: {html}"
        );
    }
}
