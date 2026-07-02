//! Minimal local HTTP writeback loop for the adjudication panel (backlog 005).
//!
//! [`crate::adjudication_panel`]'s static render has always been "just a
//! projection... no store, no server, no hidden write path." That is exactly
//! why zero real human labels have ever existed: the Keep/Nit/Wrong/Noise
//! buttons were pure CSS. This module is the other half — no framework, a
//! `std::net` request loop is enough for one local human's session:
//!
//! - `GET /` and `GET /queue.json` serve [`crate::adjudication_panel::render_live`]
//!   and the current queue (including labels applied so far this session).
//! - `POST /label` takes `{finding_id, verdict, in_scope, latency_ms}`, mints
//!   a [`Label`] through the same [`apply_label`] path `crucible adjudicate
//!   --apply` uses, and persists the accumulated labels as a
//!   `crucible.label.v1` JSON array — the exact shape `--apply` reads back, so
//!   a session's output re-enters the headless loop with no conversion.
//!
//! Single-threaded, one connection at a time: this serves one judge clicking
//! buttons, not concurrent traffic. Binds `127.0.0.1` only.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

use anyhow::Context;
use crucible_core::{apply_label, Disposition, JudgmentQueue, Label, LabelConditions, Verdict};
use serde::Deserialize;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::load_queue;

/// What to serve and where to persist labels.
pub struct ServeOptions {
    pub queue_path: PathBuf,
    pub labels_path: PathBuf,
    pub port: u16,
}

/// Bind and serve until the process is killed (Ctrl-C). Loads the queue once;
/// resumes any labels already at `labels_path` so a session can be stopped and
/// restarted without losing work.
pub fn serve(opts: ServeOptions) -> anyhow::Result<()> {
    let queue = load_queue(&opts.queue_path)?;
    let mut labels = load_existing_labels(&opts.labels_path)?;
    // Every resumed label must still name a real item in this queue — a
    // labels file resumed against a different/stale queue is a data bug the
    // server should refuse to serve silently over.
    for label in &labels {
        queue.item(&label.finding_id).with_context(|| {
            format!(
                "{} names finding id {:?}, which is not in {}",
                opts.labels_path.display(),
                label.finding_id,
                opts.queue_path.display()
            )
        })?;
    }

    let listener = TcpListener::bind(("127.0.0.1", opts.port))
        .with_context(|| format!("binding 127.0.0.1:{}", opts.port))?;
    let bound_port = listener.local_addr().map(|a| a.port()).unwrap_or(opts.port);
    println!(
        "crucible adjudication-panel: serving http://127.0.0.1:{bound_port} ({} item(s), {} labeled) — Ctrl-C to stop",
        queue.items.len(),
        labels.len()
    );

    accept_loop(listener, &queue, &mut labels, &opts.labels_path);
    Ok(())
}

/// The blocking one-connection-at-a-time accept loop, factored out of
/// [`serve`] so tests can run it against an ephemeral `127.0.0.1:0` listener
/// over a real `TcpStream`, not just call the request handlers directly.
fn accept_loop(
    listener: TcpListener,
    queue: &JudgmentQueue,
    labels: &mut Vec<Label>,
    labels_path: &Path,
) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("adjudication-panel: accept error: {err:#}");
                continue;
            }
        };
        if let Err(err) = handle_connection(stream, queue, labels, labels_path) {
            eprintln!("adjudication-panel: connection error: {err:#}");
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    queue: &JudgmentQueue,
    labels: &mut Vec<Label>,
    labels_path: &Path,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .context("cloning connection for reading")?,
    );

    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .context("reading request line")?;
    if request_line.is_empty() {
        return Ok(()); // client closed before sending anything
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut content_length: usize = 0;
    loop {
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .context("reading request headers")?;
        let trimmed = header_line.trim_end_matches(['\r', '\n']);
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

    match (method.as_str(), path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => {
            let mut render_queue = queue.clone();
            render_queue.labels = labels.clone();
            let html = crate::adjudication_panel::render_live(&render_queue);
            respond(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                html.as_bytes(),
            )
        }
        ("GET", "/queue.json") => {
            let mut render_queue = queue.clone();
            render_queue.labels = labels.clone();
            let json = serde_json::to_vec_pretty(&render_queue).context("serializing queue")?;
            respond(&mut stream, 200, "application/json", &json)
        }
        ("POST", "/label") => match handle_label_post(&body, queue, labels, labels_path) {
            Ok(response) => respond(&mut stream, 200, "application/json", &response),
            Err(err) => {
                let body = serde_json::json!({ "error": err.to_string() });
                respond(
                    &mut stream,
                    400,
                    "application/json",
                    serde_json::to_string(&body).unwrap_or_default().as_bytes(),
                )
            }
        },
        _ => respond(&mut stream, 404, "text/plain; charset=utf-8", b"not found"),
    }
}

#[derive(Debug, Deserialize)]
struct LabelRequest {
    finding_id: String,
    verdict: Verdict,
    #[serde(default = "default_in_scope")]
    in_scope: bool,
    #[serde(default)]
    latency_ms: u64,
}

fn default_in_scope() -> bool {
    true
}

/// Mint and persist one label from a `POST /label` body. `finding_id` must
/// name a real item in the queue — the same "not an adjudication item" guard
/// `crucible adjudicate --apply` enforces — and a repeat verdict on the same
/// finding replaces the prior one (last-write-wins, matching
/// [`crucible_core::judgment::reconcile_labels`]'s semantics) rather than
/// accumulating duplicates. `saw_grader_before_commit` is always `true`: the
/// panel shows the deterministic grader's context before every verdict.
fn handle_label_post(
    body: &[u8],
    queue: &JudgmentQueue,
    labels: &mut Vec<Label>,
    labels_path: &Path,
) -> anyhow::Result<Vec<u8>> {
    let request: LabelRequest =
        serde_json::from_slice(body).context("parsing label request body as JSON")?;
    let item = queue.item(&request.finding_id).with_context(|| {
        format!(
            "finding id {:?} is not an adjudication item in this queue",
            request.finding_id
        )
    })?;
    let conditions = LabelConditions {
        latency_ms: request.latency_ms,
        saw_grader_before_commit: true,
        timestamp: now_rfc3339()?,
    };
    let label = apply_label(
        item,
        request.verdict,
        Disposition {
            in_scope: request.in_scope,
        },
        &conditions,
    );

    labels.retain(|existing| existing.finding_id != label.finding_id);
    labels.push(label.clone());
    persist_labels(labels_path, labels)?;

    let response = serde_json::json!({
        "ok": true,
        "label": label,
        "labeled": labels.len(),
        "total": queue.items.len(),
    });
    serde_json::to_vec(&response).context("serializing label response")
}

fn now_rfc3339() -> anyhow::Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("formatting current timestamp")
}

fn load_existing_labels(path: &Path) -> anyhow::Result<Vec<Label>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading existing labels file {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {} as a JSON array of labels", path.display()))
}

/// Write the accumulated labels as a `crucible.label.v1` JSON array, via a
/// temp-file-then-rename so a crash mid-write cannot leave a truncated,
/// unparseable labels file behind.
fn persist_labels(path: &Path, labels: &[Label]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
    }
    let json = serde_json::to_string_pretty(labels).context("serializing labels")?;
    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, format!("{json}\n"))
        .with_context(|| format!("writing {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()))
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .context("writing response header")?;
    stream.write_all(body).context("writing response body")?;
    stream.flush().context("flushing response")
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use crucible_core::{GradeSummary, JudgmentItem, KeyFinding, Verdict, JUDGMENT_QUEUE_SCHEMA};

    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crucible-adjudication-server-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn item(id: &str) -> JudgmentItem {
        JudgmentItem {
            finding_id: id.to_string(),
            candidate: KeyFinding {
                file: "cache.py".to_string(),
                line: 23,
                category: "concurrency".to_string(),
                severity: "blocking".to_string(),
                description: "Concurrent writers share one temp file.".to_string(),
                source_id: Some(id.to_string()),
            },
            recoverable_against: Vec::new(),
        }
    }

    fn queue(ids: &[&str]) -> JudgmentQueue {
        JudgmentQueue {
            schema_version: JUDGMENT_QUEUE_SCHEMA.to_string(),
            summary: GradeSummary {
                matched: 0,
                disputed: ids.len(),
                missed: 0,
                recoverable_misses: 0,
            },
            items: ids.iter().map(|id| item(id)).collect(),
            labels: Vec::new(),
        }
    }

    #[test]
    fn handle_label_post_mints_and_persists_a_label() {
        let dir = temp_dir("mint");
        let labels_path = dir.join("labels.json");
        let queue = queue(&["F1"]);
        let mut labels = Vec::new();

        let body = serde_json::to_vec(&serde_json::json!({
            "finding_id": "F1",
            "verdict": "keep",
            "in_scope": true,
            "latency_ms": 4200
        }))
        .unwrap();
        let response = handle_label_post(&body, &queue, &mut labels, &labels_path)
            .expect("F1 is a real queue item");
        let response: serde_json::Value = serde_json::from_slice(&response).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["label"]["finding_id"], "F1");
        assert_eq!(response["label"]["verdict"], "keep");
        assert_eq!(response["label"]["latency_ms"], 4200);
        assert_eq!(
            response["label"]["saw_grader_before_commit"], true,
            "the panel always shows grader context before a verdict"
        );
        assert_eq!(response["labeled"], 1);
        assert_eq!(response["total"], 1);

        assert_eq!(labels.len(), 1);
        let persisted = load_existing_labels(&labels_path).expect("labels persisted to disk");
        assert_eq!(persisted, labels);
    }

    #[test]
    fn handle_label_post_rejects_an_unknown_finding_id() {
        let dir = temp_dir("unknown");
        let labels_path = dir.join("labels.json");
        let queue = queue(&["F1"]);
        let mut labels = Vec::new();

        let body = serde_json::to_vec(&serde_json::json!({
            "finding_id": "F999",
            "verdict": "wrong"
        }))
        .unwrap();
        let err = handle_label_post(&body, &queue, &mut labels, &labels_path)
            .expect_err("F999 is not a queue item");
        assert!(err.to_string().contains("F999"));
        assert!(labels.is_empty());
        assert!(
            !labels_path.exists(),
            "a rejected label must not be persisted"
        );
    }

    #[test]
    fn handle_label_post_last_write_wins_on_a_repeat_verdict() {
        let dir = temp_dir("repeat");
        let labels_path = dir.join("labels.json");
        let queue = queue(&["F1"]);
        let mut labels = Vec::new();

        let first =
            serde_json::to_vec(&serde_json::json!({"finding_id": "F1", "verdict": "wrong"}))
                .unwrap();
        handle_label_post(&first, &queue, &mut labels, &labels_path).unwrap();
        let second =
            serde_json::to_vec(&serde_json::json!({"finding_id": "F1", "verdict": "keep"}))
                .unwrap();
        handle_label_post(&second, &queue, &mut labels, &labels_path).unwrap();

        assert_eq!(
            labels.len(),
            1,
            "the corrected verdict replaces, not appends"
        );
        assert_eq!(labels[0].verdict, Verdict::Keep);
    }

    #[test]
    fn load_existing_labels_returns_empty_for_a_missing_file() {
        let dir = temp_dir("missing");
        let labels = load_existing_labels(&dir.join("does-not-exist.json")).unwrap();
        assert!(labels.is_empty());
    }

    /// The live loop over a real socket: bind an ephemeral port, drive the
    /// actual accept loop in a background thread, and issue real HTTP
    /// requests over `TcpStream` — not a call into the handler functions
    /// directly. Proves the wire format (headers, Content-Length framing)
    /// round-trips, not just the Rust-level logic.
    #[test]
    fn live_server_serves_the_panel_and_accepts_a_real_http_label_post() {
        let dir = temp_dir("live");
        let labels_path = dir.join("labels.json");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().unwrap().port();
        let queue_for_thread = queue(&["F1"]);
        let labels_path_for_thread = labels_path.clone();
        std::thread::spawn(move || {
            let mut labels = Vec::new();
            accept_loop(
                listener,
                &queue_for_thread,
                &mut labels,
                &labels_path_for_thread,
            );
        });

        let get_response = http_request(port, "GET /queue.json HTTP/1.1\r\nHost: local\r\n\r\n");
        assert!(
            get_response.starts_with("HTTP/1.1 200 OK"),
            "{get_response}"
        );
        assert!(get_response.contains("\"F1\""), "{get_response}");

        let label_body = serde_json::to_string(&serde_json::json!({
            "finding_id": "F1",
            "verdict": "nit",
            "in_scope": true,
            "latency_ms": 1500
        }))
        .unwrap();
        let post_request = format!(
            "POST /label HTTP/1.1\r\nHost: local\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{label_body}",
            label_body.len()
        );
        let post_response = http_request(port, &post_request);
        assert!(
            post_response.starts_with("HTTP/1.1 200 OK"),
            "{post_response}"
        );
        assert!(post_response.contains("\"labeled\":1"), "{post_response}");

        let unknown_request = "GET /nope HTTP/1.1\r\nHost: local\r\n\r\n";
        let unknown_response = http_request(port, unknown_request);
        assert!(
            unknown_response.starts_with("HTTP/1.1 404"),
            "{unknown_response}"
        );

        let persisted =
            load_existing_labels(&labels_path).expect("labels persisted by the live server");
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].verdict, Verdict::Nit);
    }

    fn http_request(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to live server");
        stream.write_all(request.as_bytes()).expect("write request");
        stream.flush().ok();
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("read response until the server closes the connection");
        response
    }
}
