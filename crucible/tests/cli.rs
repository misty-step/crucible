//! End-to-end CLI tests, driving the built `crucible` binary as a subprocess
//! (no `assert_cmd` dependency — Cargo hands integration tests the binary path
//! in `CARGO_BIN_EXE_crucible`).
//!
//! The artifact under test is a byte-identical copy of the real Cerberus
//! self-review artifact (`cerberus/evidence/self-review-001/artifact.json`),
//! kept in `tests/fixtures/` so the suite is hermetic. Its single finding is a
//! `security` finding anchored at `src/harness.rs:349`; `tests/fixtures/key.json`
//! expects that finding plus one the review never raised, so a grade over the
//! pair is a non-trivial 1 matched / 0 disputed / 1 missed.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde_json::json;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn repo_fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crucible crate has a workspace parent")
        .join(path)
}

fn crucible() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crucible"))
}

fn write_jsonrpc(stdin: &mut ChildStdin, message: serde_json::Value) {
    writeln!(stdin, "{message}").expect("write JSON-RPC message");
    stdin.flush().expect("flush JSON-RPC stdin");
}

fn read_jsonrpc(stdout: &mut BufReader<ChildStdout>) -> serde_json::Value {
    let mut line = String::new();
    stdout.read_line(&mut line).expect("read JSON-RPC response");
    assert!(!line.is_empty(), "MCP server closed stdout unexpectedly");
    serde_json::from_str(&line).expect("MCP response is JSON")
}

fn temp_root(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("crucible-cli-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp root");
    dir
}

fn http_get(port: u16, path: &str) -> String {
    let response = http_request(port, "GET", path, &[], "");
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "expected 200 for {path}, got {response}"
    );
    response_body(&response)
}

fn http_get_auth(port: u16, path: &str, bearer: &str) -> String {
    let auth = format!("Bearer {bearer}");
    let response = http_request(port, "GET", path, &[("Authorization", auth.as_str())], "");
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "expected 200 for {path}, got {response}"
    );
    response_body(&response)
}

fn http_post_json(port: u16, path: &str, bearer: Option<&str>, body: &str) -> String {
    let auth = bearer.map(|bearer| format!("Bearer {bearer}"));
    let mut headers = vec![("Content-Type", "application/json")];
    if let Some(auth) = auth.as_deref() {
        headers.push(("Authorization", auth));
    }
    http_request(port, "POST", path, &headers, body)
}

fn http_request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to crucible serve");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set read timeout");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n"
    )
    .expect("write HTTP request");
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").expect("write HTTP header");
    }
    if !body.is_empty() {
        write!(stream, "Content-Length: {}\r\n", body.len()).expect("write content length");
    }
    write!(stream, "\r\n{body}").expect("write HTTP body");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish HTTP request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read HTTP response");
    response
}

fn response_body(response: &str) -> String {
    response
        .split("\r\n\r\n")
        .nth(1)
        .expect("HTTP response has a body")
        .to_string()
}

fn http_get_json(port: u16, path: &str) -> serde_json::Value {
    serde_json::from_str(&http_get(port, path)).expect("HTTP response body is JSON")
}

fn http_get_json_auth(port: u16, path: &str, bearer: &str) -> serde_json::Value {
    serde_json::from_str(&http_get_auth(port, path, bearer)).expect("HTTP response body is JSON")
}

fn spawn_serve(db: &Path, specs: &Path, bearer: Option<&str>) -> (Child, u16) {
    let mut command = crucible();
    command
        .arg("serve")
        .arg("--db")
        .arg(db)
        .arg("--specs")
        .arg(specs)
        .arg("--port")
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match bearer {
        Some(bearer) => {
            command.env("CRUCIBLE_SERVE_TOKEN", bearer);
        }
        None => {
            command.env_remove("CRUCIBLE_SERVE_TOKEN");
        }
    }
    let mut child = command.spawn().expect("spawn crucible serve");
    let stdout = child.stdout.take().expect("serve stdout is piped");
    let mut stdout = BufReader::new(stdout);
    let mut line = String::new();
    stdout
        .read_line(&mut line)
        .expect("read serve startup line");
    let port: u16 = line
        .split("http://127.0.0.1:")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("startup line names bound localhost port: {line:?}"));
    (child, port)
}

fn stop_child(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The headline test: `grade --json` over the real artifact emits a stable,
/// parseable JSON object with the expected partition and a bracketing Wilson
/// interval on the match rate.
#[test]
fn grade_emits_valid_json_over_the_real_artifact() {
    let out = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "grade must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("grade --json must emit valid JSON");

    assert_eq!(
        v["schema_version"], "crucible.grade_report.v1",
        "grade --json carries a stable schema id for headless parsers: {v}"
    );

    // The real finding matches its key row; the prompt.rs row is missed.
    assert_eq!(v["matched"], 1, "the harness finding matches its key row");
    assert_eq!(v["disputed"], 0, "the only candidate matched");
    assert_eq!(v["missed"], 1, "the prompt.rs key row is unfound");
    assert_eq!(v["dropped_invalid"], 0, "the lone finding is schema-valid");

    // Wilson interval over the match rate matched / (matched + missed) = 1/2.
    let rate = &v["match_rate"];
    assert_eq!(rate["successes"], 1);
    assert_eq!(rate["n"], 2);

    let point = rate["point"].as_f64().expect("point is a number");
    assert!(
        (point - 0.5).abs() < 1e-9,
        "point estimate {point} should be 0.5"
    );

    let lower = rate["lower"].as_f64().expect("lower is a number");
    let upper = rate["upper"].as_f64().expect("upper is a number");
    assert!(
        lower < point && point < upper,
        "interval [{lower}, {upper}] must bracket {point}"
    );
    assert!(
        (0.0..=1.0).contains(&lower) && (0.0..=1.0).contains(&upper),
        "Wilson bounds [{lower}, {upper}] must stay within [0, 1]"
    );
}

/// `adapt --json` projects the artifact's one finding onto an answer-key row.
#[test]
fn adapt_emits_valid_json_listing_the_mapped_finding() {
    let out = crucible()
        .arg("adapt")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "adapt must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("adapt --json must emit valid JSON");

    assert_eq!(
        v["schema_version"], "crucible.adapt_report.v1",
        "adapt --json carries a stable schema id for headless parsers: {v}"
    );
    assert_eq!(v["count"], 1);
    let findings = v["findings"].as_array().expect("findings is an array");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["file"], "src/harness.rs");
    assert_eq!(findings[0]["line"], 349);
    assert_eq!(findings[0]["category"], "security");
    assert_eq!(
        findings[0]["severity"], "minor",
        "Minor collapses to 'minor'"
    );
}

/// Human (non-`--json`) grade output is a readable summary and still exits 0.
#[test]
fn grade_human_mode_exits_zero_and_reports_counts() {
    let out = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key.json"))
        .output()
        .expect("crucible binary runs");

    assert!(out.status.success(), "human grade must exit 0");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("matched"), "summary names the counts: {text}");
    assert!(
        text.contains("match rate"),
        "summary shows the rate: {text}"
    );
}

/// Human (non-`--json`) adapt output renders the mapped location and exits 0.
#[test]
fn adapt_human_mode_exits_zero_and_shows_the_location() {
    let out = crucible()
        .arg("adapt")
        .arg(fixture("cerberus-artifact.json"))
        .output()
        .expect("crucible binary runs");

    assert!(out.status.success(), "human adapt must exit 0");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("src/harness.rs:349"),
        "table shows the mapped location: {text}"
    );
}

/// A missing artifact is a hard error, not a silent exit-0.
#[test]
fn grade_with_missing_artifact_fails() {
    let out = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg("/no/such/crucible/artifact.json")
        .arg("--key")
        .arg(fixture("key.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        !out.status.success(),
        "a missing artifact must produce a non-zero exit"
    );
}

/// An empty answer key (`{ "findings": [] }`) is a valid grade with no key rows:
/// the match rate has no denominator. JSON must report `n == 0` and a `null`
/// point ("no data" is not "0%"), and the human view must print `n/a` rather than
/// a bogus `0%`.
#[test]
fn grade_with_empty_key_reports_na_and_null_point() {
    let out = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("empty-key.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "empty-key grade must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("grade --json must emit valid JSON");

    assert_eq!(v["missed"], 0, "an empty key has no rows to miss");
    let rate = &v["match_rate"];
    assert_eq!(rate["n"], 0, "no key rows -> n == 0");
    assert!(
        rate["point"].is_null(),
        "point is null (not 0.0) when n == 0: {v}"
    );

    let human = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("empty-key.json"))
        .output()
        .expect("crucible binary runs");
    assert!(human.status.success(), "human empty-key grade must exit 0");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("n/a"),
        "empty key renders the rate as n/a: {text}"
    );
}

/// Finding-2 regression at the CLI: roughly half of real Daedalus keys omit
/// `severity`. `grade` must *load* such a key (exit 0) rather than hard-erroring.
/// This key (real, severity-less) shares no location with the artifact, so the
/// review's lone finding lands in `disputed` — also pinning the `disputed > 0`
/// rendering the other tests never exercise.
#[test]
fn grade_loads_severity_less_key_and_reports_disputed() {
    let out = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key-severity-less.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "a severity-less key must load, not hard-error; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("grade --json must emit valid JSON");

    let disputed = v["disputed"].as_u64().expect("disputed is a number");
    assert!(
        disputed >= 1,
        "the review's finding is absent from this key, so it is disputed: {v}"
    );
}

/// `grade --key` must read a `tests/expected.json` **defects** key — the span
/// key `daedalus-score` scores against — not just a `solution/findings.json`.
/// The defect co-located with the review's finding (`src/harness.rs:349`,
/// security) is matched, so the grade reports `matched >= 1`, proving the key
/// loaded rather than silently grading against zero rows (the original defect).
#[test]
fn grade_reads_a_defects_format_scorer_key() {
    let out = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("expected-defects.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "grade must read a defects key and exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("grade --json must emit valid JSON");
    assert_eq!(
        v["matched"], 1,
        "the review's finding matches the co-located defect (key loaded, not 0 rows): {v}"
    );
    assert_eq!(
        v["missed"], 1,
        "the other defect is at a location the review never raised"
    );
}

/// A `--key` file that is neither a `findings` nor a `defects` array is a hard
/// error (exit 1) with a message naming both expected shapes — not a silent
/// exit-0 grading against zero key rows.
#[test]
fn grade_rejects_a_key_with_neither_findings_nor_defects() {
    let out = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key-bad-shape.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert_eq!(
        out.status.code(),
        Some(1),
        "an unrecognized key shape is a load error, exit 1"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("defects") && stderr.contains("findings"),
        "the error names both expected key shapes: {stderr}"
    );
}

/// Finding-3 surfacing: a key row at the SAME location as the review's finding
/// but a different category vocabulary is category-blocked — the candidate is
/// `disputed`, the key `missed`. The grade must expose this as a
/// `recoverable_misses` so the recall is not read as a final rate, and the human
/// view must carry the caveat.
#[test]
fn grade_surfaces_recoverable_misses_for_colocated_category_mismatch() {
    let out = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key-colocated-other-category.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(out.status.success(), "grade must exit 0");
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("grade --json must emit valid JSON");

    assert_eq!(v["matched"], 0, "the category mismatch blocks the match");
    assert!(
        v["disputed"].as_u64().unwrap() >= 1,
        "candidate is disputed"
    );
    assert!(v["missed"].as_u64().unwrap() >= 1, "key row is missed");
    assert!(
        v["recoverable_misses"].as_u64().unwrap() >= 1,
        "the co-located miss is recoverable: {v}"
    );

    let human = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key-colocated-other-category.json"))
        .output()
        .expect("crucible binary runs");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("category vocabulary mismatch"),
        "human output carries the recoverable-miss caveat: {text}"
    );
}

/// The empty-findings adapt branch: an artifact with no findings maps to zero
/// rows and prints the `(no findings)` placeholder rather than an empty table.
#[test]
fn adapt_of_empty_artifact_reports_no_findings() {
    let out = crucible()
        .arg("adapt")
        .arg(fixture("empty-artifact.json"))
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "adapt of an empty artifact must exit 0"
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("(no findings)"),
        "empty adapt shows the placeholder: {text}"
    );
}

/// `grade` surfaces `dropped_invalid` — findings the schema-valid filter removed
/// before grading — in both modes. The fixture has one valid finding (matched)
/// and one malformed one (empty category/content, out-of-range confidence) that
/// must be counted as dropped, not silently swallowed into a low match count.
#[test]
fn grade_surfaces_dropped_invalid_count() {
    let out = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("artifact-invalid-finding.json"))
        .arg("--key")
        .arg(fixture("key.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(out.status.success(), "grade must exit 0");
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("grade --json must emit valid JSON");
    assert_eq!(
        v["dropped_invalid"], 1,
        "the malformed finding is dropped: {v}"
    );
    assert_eq!(
        v["matched"], 1,
        "the valid finding still matches its key row"
    );
    assert_eq!(
        v["disputed"], 0,
        "the dropped finding never reaches disputed"
    );

    let human = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("artifact-invalid-finding.json"))
        .arg("--key")
        .arg(fixture("key.json"))
        .output()
        .expect("crucible binary runs");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("dropped") && text.contains('1'),
        "human grade names the dropped count: {text}"
    );
}

/// `adjudicate --json` over a co-located category mismatch builds the queue: the
/// disputed candidate becomes the one item, traced back to its source finding id
/// (`F1`), carrying the recoverable miss as context and a stamped schema.
#[test]
fn adjudicate_emits_judgment_queue_with_traceable_recoverable_item() {
    let out = crucible()
        .arg("adjudicate")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key-colocated-other-category.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "adjudicate must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("adjudicate --json must emit valid JSON");

    assert_eq!(v["schema_version"], "crucible.judgment_queue.v1");
    assert_eq!(v["summary"]["disputed"], 1);
    assert_eq!(v["summary"]["recoverable_misses"], 1);

    let items = v["items"].as_array().expect("items is an array");
    assert_eq!(
        items.len(),
        1,
        "the lone disputed candidate is the one item"
    );
    assert_eq!(
        items[0]["finding_id"], "F1",
        "the item traces back to its Cerberus source finding"
    );
    assert_eq!(items[0]["candidate"]["source_id"], "F1");
    let recoverable = items[0]["recoverable_against"]
        .as_array()
        .expect("recoverable_against is an array");
    assert_eq!(
        recoverable.len(),
        1,
        "the co-located key row rides along as recoverable context"
    );

    // A freshly built queue carries no labels yet.
    assert!(
        v.get("labels").is_none(),
        "unlabeled queue omits labels: {v}"
    );
}

/// `adjudicate --apply` applies a labels file to the queue and emits the labeled
/// judgment artifact: each decision is validated against the queue and re-minted
/// as an append-only `Label` carrying the finding id and the conditions it was
/// committed under.
#[test]
fn adjudicate_apply_attaches_minted_labels() {
    let out = crucible()
        .arg("adjudicate")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key-colocated-other-category.json"))
        .arg("--apply")
        .arg(fixture("labels-keep-f1.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "adjudicate --apply must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("adjudicate --json must emit valid JSON");

    let labels = v["labels"].as_array().expect("labels is an array");
    assert_eq!(labels.len(), 1, "one decision applied: {v}");
    assert_eq!(labels[0]["finding_id"], "F1");
    assert_eq!(labels[0]["verdict"], "keep");
    assert_eq!(labels[0]["disposition"]["in_scope"], true);
    assert_eq!(
        labels[0]["latency_ms"], 1500,
        "conditions ride onto the label"
    );
    assert_eq!(
        labels[0]["schema_version"], "crucible.label.v1",
        "apply re-mints with the canonical label schema"
    );
}

/// A decision for a finding that is not an adjudication item is rejected, not
/// silently dropped: `--apply` with an unknown finding id is a load/parse-class
/// failure (exit 1).
#[test]
fn adjudicate_apply_rejects_unknown_finding_id() {
    let out = crucible()
        .arg("adjudicate")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key-colocated-other-category.json"))
        .arg("--apply")
        .arg(fixture("labels-unknown-id.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert_eq!(
        out.status.code(),
        Some(1),
        "an unknown finding id is a load-error exit 1"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("F404"),
        "the error names the offending finding id: {stderr}"
    );
}

/// Two decisions for the SAME finding in one `--apply` batch are reconciled to
/// the latest (append-only correction semantics): the output carries one minted
/// label for `F1`, the later-timestamp `noise` ruling, not two.
#[test]
fn adjudicate_apply_reconciles_duplicate_finding_ids_last_write_wins() {
    let out = crucible()
        .arg("adjudicate")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key-colocated-other-category.json"))
        .arg("--apply")
        .arg(fixture("labels-duplicate-f1.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "adjudicate --apply must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("adjudicate --json must emit valid JSON");

    let labels = v["labels"].as_array().expect("labels is an array");
    assert_eq!(
        labels.len(),
        1,
        "the two F1 decisions collapse to one minted label: {v}"
    );
    assert_eq!(labels[0]["finding_id"], "F1");
    assert_eq!(
        labels[0]["verdict"], "noise",
        "the later-timestamp correction wins, not the first decision: {v}"
    );
    assert_eq!(labels[0]["disposition"]["in_scope"], false);
}

/// A `--key` that is a top-level JSON array (not the expected object with a
/// `findings`/`defects` array) is a hard error (exit 1) naming the structural
/// mismatch — never a silent grade against zero key rows.
#[test]
fn grade_rejects_a_top_level_array_key() {
    let out = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key-array.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert_eq!(
        out.status.code(),
        Some(1),
        "a top-level array key is a load error, exit 1"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("top-level JSON array"),
        "the error names the structural mismatch: {stderr}"
    );
}

/// The human `adjudicate` view renders the queue table with the item's id, its
/// recoverable kind, and the mapped location.
#[test]
fn adjudicate_human_mode_renders_the_queue_table() {
    let out = crucible()
        .arg("adjudicate")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key-colocated-other-category.json"))
        .output()
        .expect("crucible binary runs");

    assert!(out.status.success(), "human adjudicate must exit 0");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("queue item"), "names the queue: {text}");
    assert!(text.contains("F1"), "shows the item id: {text}");
    assert!(
        text.contains("recoverable"),
        "marks the recoverable kind: {text}"
    );
    assert!(
        text.contains("src/harness.rs:349"),
        "shows the mapped location: {text}"
    );
}

/// `crucible run` is the runnable-evals contract for cold agents: one command
/// writes three concrete eval receipts, each with a defensible score shape and
/// inspectable artifacts.
#[test]
fn run_all_writes_three_runnable_eval_receipts() {
    let out_dir = temp_root("run-all");
    let db = out_dir.join("runs.sqlite");
    let out = crucible()
        .arg("run")
        .arg("--out")
        .arg(&out_dir)
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "run must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("run --json must emit valid JSON");
    assert_eq!(v["schema_version"], "crucible.run_report.v1");
    let evals = v["evals"].as_array().expect("evals is an array");
    assert_eq!(evals.len(), 3, "three concrete evals run: {v}");

    for eval in evals {
        let score = &eval["score"];
        assert_eq!(
            score["method"], "Wilson",
            "every built-in eval reports a Wilson interval: {eval}"
        );
        assert!(
            score["n"].as_u64().expect("n is a count") >= 1,
            "each eval has a denominator: {eval}"
        );
        assert!(
            score["lower"].as_f64().unwrap() <= score["upper"].as_f64().unwrap(),
            "interval is ordered: {eval}"
        );
    }

    assert!(out_dir.join("run-report.json").exists());
    assert!(out_dir
        .join("code-review-deterministic-floor")
        .join("grade.json")
        .exists());
    assert!(out_dir
        .join("recoverable-adjudication-queue")
        .join("panel")
        .join("index.html")
        .exists());
    assert!(out_dir
        .join("harbor-export-acceptance")
        .join("tests")
        .join("expected.json")
        .exists());
    let expected: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            out_dir
                .join("harbor-export-acceptance")
                .join("tests")
                .join("expected.json"),
        )
        .expect("read exported scorer key"),
    )
    .expect("scorer key is JSON");
    let oracle: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            out_dir
                .join("harbor-export-acceptance")
                .join("solution")
                .join("findings.json"),
        )
        .expect("read exported oracle key"),
    )
    .expect("oracle key is JSON");
    assert_eq!(
        expected["defects"].as_array().expect("defects array").len(),
        1,
        "one accepted finding reaches the scorer key"
    );
    assert_eq!(
        oracle["findings"].as_array().expect("findings array").len(),
        1,
        "the same accepted finding reaches the oracle key"
    );
}

/// `crucible run <spec>` is the declared-eval path: the command loads an
/// `EvalSpec`, executes its runner, and writes the same scored run-report shape
/// as built-in receipts.
#[test]
fn run_declared_spec_writes_a_scored_key_recall_report() {
    let out_dir = temp_root("run-spec");
    let db = out_dir.join("runs.sqlite");
    let spec = fixture("specs/key-recall-fixture.json");
    let out = crucible()
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&out_dir)
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "declared spec run must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("run <spec> --json emits JSON");
    assert_eq!(v["schema_version"], "crucible.run_report.v1");
    assert_eq!(v["evals"].as_array().expect("evals array").len(), 1);
    let eval = &v["evals"][0];
    assert_eq!(eval["id"], "key-recall-fixture");
    assert_eq!(eval["score"]["metric"], "pr_review_key_recall");
    assert_eq!(eval["score"]["successes"], 1);
    assert_eq!(eval["score"]["n"], 2);
    assert_eq!(eval["score"]["method"], "Wilson");
    assert!(
        eval["score"]["lower"].as_f64().unwrap() < 0.5
            && 0.5 < eval["score"]["upper"].as_f64().unwrap(),
        "Wilson interval brackets the 1/2 point estimate: {eval}"
    );

    let evidence_path = out_dir.join("task-results.json");
    assert!(out_dir.join("run-report.json").exists());
    assert!(evidence_path.exists(), "task-level evidence written");
    let evidence: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(evidence_path).expect("read evidence"))
            .expect("evidence is JSON");
    assert_eq!(evidence["schema_version"], "crucible.spec_run_evidence.v1");
    assert_eq!(evidence["totals"]["matched"], 1);
    assert_eq!(evidence["totals"]["expected_defects"], 2);
    assert_eq!(evidence["tasks"].as_array().expect("tasks array").len(), 1);
}

/// `crucible run` now writes to a SQLite run ledger, and `crucible runs` can
/// query the run by benchmark, run id, and config/model comparison even if the
/// loose run-report JSON is no longer present.
#[test]
fn run_persists_to_sqlite_and_cli_queries_the_ledger() {
    let root = temp_root("run-db");
    let out_dir = root.join("out");
    let db = root.join("runs.sqlite");
    let spec = fixture("specs/key-recall-fixture.json");

    let out = crucible()
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&out_dir)
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert!(
        out.status.success(),
        "declared spec run must persist; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let list = crucible()
        .arg("runs")
        .arg("list")
        .arg("--db")
        .arg(&db)
        .arg("--benchmark")
        .arg("key-recall-fixture")
        .arg("--json")
        .output()
        .expect("crucible runs list executes");
    assert!(
        list.status.success(),
        "runs list exits 0; stderr: {}",
        String::from_utf8_lossy(&list.stderr)
    );
    let list: serde_json::Value =
        serde_json::from_slice(&list.stdout).expect("runs list emits JSON");
    assert_eq!(list["schema_version"], "crucible.run_store.v1");
    let runs = list["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["benchmark_id"], "key-recall-fixture");
    assert_eq!(runs[0]["runner_kind"], "key_recall");
    assert_eq!(runs[0]["config_id"], "probe");
    assert_eq!(runs[0]["score_metric"], "pr_review_key_recall");
    let run_id = runs[0]["run_id"].as_str().expect("run id").to_string();

    std::fs::remove_file(out_dir.join("run-report.json")).expect("remove loose run report");

    let show = crucible()
        .arg("runs")
        .arg("show")
        .arg(&run_id)
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .output()
        .expect("crucible runs show executes");
    assert!(
        show.status.success(),
        "runs show exits 0 after loose report deletion; stderr: {}",
        String::from_utf8_lossy(&show.stderr)
    );
    let show: serde_json::Value =
        serde_json::from_slice(&show.stdout).expect("runs show emits JSON");
    assert_eq!(show["run"]["run_id"], run_id);
    assert_eq!(show["eval_json"]["id"], "key-recall-fixture");
    assert_eq!(show["artifacts"].as_array().expect("artifacts").len(), 2);
    assert_eq!(
        show["prompt_tasks"].as_array().expect("prompt tasks").len(),
        0,
        "key-recall runs have no prompt task rows"
    );
    assert_eq!(
        show["run_record"]["schema_version"],
        "crucible.run_record.v1"
    );
    assert_eq!(
        show["run_record"]["score"]["metric"],
        "pr_review_key_recall"
    );
    assert_eq!(
        show["evaluation_card"]["schema_version"],
        "crucible.evaluation_card.v1"
    );
    assert_eq!(
        show["evaluation_card"]["provenance"]["model"],
        "deterministic"
    );
    assert_eq!(
        show["evaluation_card"],
        show["run_record"]["evaluation_card"]
    );

    let findings_out = root.join("findings.json");
    let compare = crucible()
        .arg("runs")
        .arg("compare")
        .arg("--db")
        .arg(&db)
        .arg("--benchmark")
        .arg("key-recall-fixture")
        .arg("--left")
        .arg("probe")
        .arg("--right")
        .arg("probe")
        .arg("--findings-out")
        .arg(&findings_out)
        .arg("--json")
        .output()
        .expect("crucible runs compare executes");
    assert!(
        compare.status.success(),
        "runs compare exits 0; stderr: {}",
        String::from_utf8_lossy(&compare.stderr)
    );
    let compare: serde_json::Value =
        serde_json::from_slice(&compare.stdout).expect("runs compare emits JSON");
    assert_eq!(
        compare["comparison_kind"],
        "latest_unpaired_descriptive_delta"
    );
    assert_eq!(compare["delta_point"], 0.0);
    let findings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&findings_out).expect("read findings"))
            .expect("findings journal is JSON");
    assert_eq!(findings["schema_version"], "crucible.findings_journal.v1");
    assert_eq!(
        findings["findings"]
            .as_array()
            .expect("findings is an array")
            .len(),
        0,
        "unpaired descriptive deltas must not mint finding records"
    );
}

/// `crucible runs list` filters by config/model/date, and `runs compare
/// --alpha` threads the significance threshold through to the CLI, over the
/// real binary (not just the `run_store` unit tests).
#[test]
fn runs_list_filters_by_config_model_and_date() {
    let root = temp_root("run-db-filters");
    let db = root.join("runs.sqlite");
    let spec = fixture("specs/key-recall-fixture.json");

    // A second declared spec with its own corpus copy (an added "probe-2"
    // trial and matching key), so it persists under a distinct `config_id`
    // without touching the shared fixture tree.
    let corpus_2 = root.join("spec-corpus-2");
    let arena_2 = corpus_2.join("arena");
    std::fs::create_dir_all(&corpus_2).expect("create probe-2 corpus dir");
    let source_arena = fixture("spec-corpus/arena");
    let copy_status = Command::new("cp")
        .arg("-R")
        .arg(&source_arena)
        .arg(&arena_2)
        .status()
        .expect("cp arena dir for probe-2 corpus");
    assert!(copy_status.success(), "copying the arena dir must succeed");
    let trials_text = std::fs::read_to_string(fixture("spec-corpus/trials.jsonl"))
        .expect("read source trials.jsonl");
    let mut probe_2_trial: serde_json::Value =
        serde_json::from_str(trials_text.lines().next().expect("one trial line"))
            .expect("trial line is JSON");
    probe_2_trial["candidate_id"] = json!("probe-2");
    probe_2_trial["run_id"] = json!("fixture-run-probe-2-t1");
    std::fs::write(
        corpus_2.join("trials.jsonl"),
        format!("{trials_text}{probe_2_trial}\n"),
    )
    .expect("write probe-2 trials.jsonl");

    let spec_text = std::fs::read_to_string(&spec).expect("read fixture spec");
    let mut spec_json: serde_json::Value =
        serde_json::from_str(&spec_text).expect("fixture spec is JSON");
    spec_json["runner"]["corpus"]["candidate_id"] = json!("probe-2");
    spec_json["runner"]["corpus"]["arena_dir"] = json!(arena_2.display().to_string());
    spec_json["runner"]["corpus"]["trials_jsonl"] =
        json!(corpus_2.join("trials.jsonl").display().to_string());
    let spec_2 = root.join("key-recall-fixture-probe-2.json");
    std::fs::write(
        &spec_2,
        serde_json::to_string_pretty(&spec_json).expect("serialize probe-2 spec"),
    )
    .expect("write probe-2 spec fixture");

    let run = |spec: &Path, out_name: &str| {
        let out_dir = root.join(out_name);
        let out = crucible()
            .arg("run")
            .arg(spec)
            .arg("--out")
            .arg(&out_dir)
            .arg("--db")
            .arg(&db)
            .arg("--json")
            .output()
            .expect("crucible binary runs");
        assert!(
            out.status.success(),
            "run must persist; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&spec, "out-probe");
    run(&spec_2, "out-probe-2");

    let list_all = crucible()
        .arg("runs")
        .arg("list")
        .arg("--db")
        .arg(&db)
        .arg("--benchmark")
        .arg("key-recall-fixture")
        .arg("--json")
        .output()
        .expect("crucible runs list executes");
    let list_all: serde_json::Value =
        serde_json::from_slice(&list_all.stdout).expect("runs list emits JSON");
    assert_eq!(
        list_all["runs"].as_array().expect("runs array").len(),
        2,
        "both configs are stored under the shared benchmark"
    );

    let list_by_config = crucible()
        .arg("runs")
        .arg("list")
        .arg("--db")
        .arg(&db)
        .arg("--config")
        .arg("probe-2")
        .arg("--json")
        .output()
        .expect("crucible runs list --config executes");
    let list_by_config: serde_json::Value =
        serde_json::from_slice(&list_by_config.stdout).expect("runs list --config emits JSON");
    let filtered = list_by_config["runs"].as_array().expect("runs array");
    assert_eq!(filtered.len(), 1, "config filter narrows to one run");
    assert_eq!(filtered[0]["config_id"], "probe-2");

    let list_future_since = crucible()
        .arg("runs")
        .arg("list")
        .arg("--db")
        .arg(&db)
        .arg("--since")
        .arg("2999-01-01")
        .arg("--json")
        .output()
        .expect("crucible runs list --since executes");
    let list_future_since: serde_json::Value =
        serde_json::from_slice(&list_future_since.stdout).expect("runs list --since emits JSON");
    assert_eq!(
        list_future_since["runs"]
            .as_array()
            .expect("runs array")
            .len(),
        0,
        "a since bound in the far future excludes every run"
    );

    let list_past_until = crucible()
        .arg("runs")
        .arg("list")
        .arg("--db")
        .arg(&db)
        .arg("--until")
        .arg("2000-01-01")
        .arg("--json")
        .output()
        .expect("crucible runs list --until executes");
    let list_past_until: serde_json::Value =
        serde_json::from_slice(&list_past_until.stdout).expect("runs list --until emits JSON");
    assert_eq!(
        list_past_until["runs"]
            .as_array()
            .expect("runs array")
            .len(),
        0,
        "an until bound in the past excludes every run"
    );

    let compare = crucible()
        .arg("runs")
        .arg("compare")
        .arg("--db")
        .arg(&db)
        .arg("--benchmark")
        .arg("key-recall-fixture")
        .arg("--left")
        .arg("probe")
        .arg("--right")
        .arg("probe-2")
        .arg("--alpha")
        .arg("0.2")
        .arg("--json")
        .output()
        .expect("crucible runs compare --alpha executes");
    assert!(
        compare.status.success(),
        "runs compare --alpha exits 0; stderr: {}",
        String::from_utf8_lossy(&compare.stderr)
    );
    let compare: serde_json::Value =
        serde_json::from_slice(&compare.stdout).expect("runs compare emits JSON");
    // key-recall has no indexed prompt task rows, so this stays the unpaired
    // fallback; --alpha is exercised (accepted, parsed, threaded through) even
    // though it has no paired outcome to gate here.
    assert_eq!(
        compare["comparison_kind"],
        "latest_unpaired_descriptive_delta"
    );
    assert!(compare["paired"].is_null());
}

/// Backlog 027: `runs list --harness` narrows the ledger by the new axis
/// (zero matches here, since the fixture never records one — the same
/// smoke-level assertion `runs_list_filters_by_config_model_and_date` makes
/// for the other filters), and `runs history`/`runs pivot` expose the new
/// time-series and cross-axis query surface over the real binary and a real
/// SQLite ledger, not just the `run_store` unit tests.
#[test]
fn runs_history_and_pivot_query_the_real_ledger_over_the_cli() {
    let root = temp_root("run-db-history-pivot");
    let db = root.join("runs.sqlite");
    let spec = fixture("specs/key-recall-fixture.json");

    let run = |out_name: &str| {
        let out_dir = root.join(out_name);
        let out = crucible()
            .arg("run")
            .arg(&spec)
            .arg("--out")
            .arg(&out_dir)
            .arg("--db")
            .arg(&db)
            .arg("--json")
            .output()
            .expect("crucible binary runs");
        assert!(
            out.status.success(),
            "run must persist; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    // Two runs of the identical declared spec/config over time — exactly the
    // repeated-run shape score_history's trend line and pivot's dedup-by-model
    // both need to prove something over more than one row.
    run("out-1");
    run("out-2");

    let harness_filtered = crucible()
        .arg("runs")
        .arg("list")
        .arg("--db")
        .arg(&db)
        .arg("--harness")
        .arg("codex")
        .arg("--json")
        .output()
        .expect("crucible runs list --harness executes");
    assert!(harness_filtered.status.success());
    let harness_filtered: serde_json::Value =
        serde_json::from_slice(&harness_filtered.stdout).expect("runs list --harness emits JSON");
    assert_eq!(
        harness_filtered["runs"]
            .as_array()
            .expect("runs array")
            .len(),
        0,
        "key_recall runs never declare a harness, so any --harness filter excludes them all"
    );

    let history = crucible()
        .arg("runs")
        .arg("history")
        .arg("--db")
        .arg(&db)
        .arg("--benchmark")
        .arg("key-recall-fixture")
        .arg("--config")
        .arg("probe")
        .arg("--json")
        .output()
        .expect("crucible runs history executes");
    assert!(
        history.status.success(),
        "runs history exits 0; stderr: {}",
        String::from_utf8_lossy(&history.stderr)
    );
    let history: serde_json::Value =
        serde_json::from_slice(&history.stdout).expect("runs history emits JSON");
    assert_eq!(history["benchmark"], "key-recall-fixture");
    assert_eq!(history["config_query"], "probe");
    let points = history["points"].as_array().expect("points array");
    assert_eq!(
        points.len(),
        2,
        "both persisted runs show up in the history"
    );
    assert!(
        points[0]["created_at_unix_ms"].as_i64().unwrap()
            <= points[1]["created_at_unix_ms"].as_i64().unwrap(),
        "history is ordered oldest to newest: {points:?}"
    );

    let pivot = crucible()
        .arg("runs")
        .arg("pivot")
        .arg("--db")
        .arg(&db)
        .arg("--benchmark")
        .arg("key-recall-fixture")
        .arg("--json")
        .output()
        .expect("crucible runs pivot executes");
    assert!(
        pivot.status.success(),
        "runs pivot exits 0; stderr: {}",
        String::from_utf8_lossy(&pivot.stderr)
    );
    let pivot: serde_json::Value =
        serde_json::from_slice(&pivot.stdout).expect("runs pivot emits JSON");
    assert_eq!(pivot["benchmark"], "key-recall-fixture");
    assert!(
        pivot["harness"].is_null(),
        "harness omitted when not narrowed"
    );
    let rows = pivot["rows"].as_array().expect("rows array");
    assert_eq!(
        rows.len(),
        1,
        "key_recall runs share model=null, so both runs dedup into one pivot row: {rows:?}"
    );
    assert!(rows[0]["model"].is_null());

    let pivot_no_match = crucible()
        .arg("runs")
        .arg("pivot")
        .arg("--db")
        .arg(&db)
        .arg("--benchmark")
        .arg("key-recall-fixture")
        .arg("--harness")
        .arg("codex")
        .arg("--json")
        .output()
        .expect("crucible runs pivot --harness executes");
    assert!(pivot_no_match.status.success());
    let pivot_no_match: serde_json::Value =
        serde_json::from_slice(&pivot_no_match.stdout).expect("runs pivot --harness emits JSON");
    assert_eq!(pivot_no_match["harness"], "codex");
    assert_eq!(
        pivot_no_match["rows"].as_array().expect("rows array").len(),
        0,
        "narrowing to an unrecorded harness excludes every run"
    );
}

/// Backlog `023`: pass^k task consistency must pair through the same
/// `PairedComparison`/`DeltaVerdict` McNemar kernel `compare_configs` already
/// uses for prompt-benchmark runs — not just report each run's independent
/// Wilson point estimate. One hermetic `cerberus-review-quality-v0`-shaped
/// corpus (six tasks, `k=2` trials each) backs two `crucible runs compare`
/// calls: a 0-vs-6 discordant split (exact two-sided p ≈ 0.031 — a `signal` at
/// the default alpha) and a balanced 3-vs-3 split (p = 1.0 —
/// `inside_noise_floor`).
#[test]
fn pass_k_comparison_reports_paired_noise_floor_verdict() {
    let root = temp_root("pass-k-compare");
    let db = root.join("runs.sqlite");

    // Shared arena: six tasks, each seeded with one defect. All four
    // candidates below run against this same task/expected-key set; only
    // each candidate's own trials.jsonl rows decide whether it finds the
    // seeded defect.
    let arena = root.join("arena");
    let task_ids = ["t1", "t2", "t3", "t4", "t5", "t6"];
    for task_id in task_ids {
        let tests_dir = arena.join("tasks").join(task_id).join("tests");
        std::fs::create_dir_all(&tests_dir).expect("create task tests dir");
        std::fs::write(
            tests_dir.join("expected.json"),
            serde_json::to_string_pretty(&json!({
                "defects": [{
                    "id": "d1",
                    "file": "src/lib.rs",
                    "line_start": 10,
                    "line_end": 12,
                    "category": "correctness",
                    "note": "The candidate should find this defect."
                }]
            }))
            .unwrap(),
        )
        .expect("write expected.json");
    }

    let matching_finding = json!({
        "file": "src/lib.rs",
        "line": 11,
        "category": "correctness",
        "description": "Found it."
    });

    // candidate_id -> the tasks that candidate fully matches on every trial
    // (pass^2 success); every other task in `task_ids` gets an empty findings
    // list on every trial (a missed defect, so pass^2 fails that task).
    let candidates: [(&str, &[&str]); 4] = [
        ("signal-left", &[]),                 // fails all six tasks
        ("signal-right", &task_ids),          // passes all six tasks
        ("noise-left", &["t1", "t2", "t3"]),  // passes the first half
        ("noise-right", &["t4", "t5", "t6"]), // passes the second half
    ];

    let mut trials = String::new();
    for (candidate_id, passing_tasks) in candidates {
        for task_id in task_ids {
            let passes = passing_tasks.contains(&task_id);
            for trial in 1..=2u64 {
                let findings = if passes {
                    json!([matching_finding.clone()])
                } else {
                    json!([])
                };
                let line = json!({
                    "run_id": format!("fixture-{candidate_id}-{task_id}-{trial}"),
                    "arena_id": "fixture-arena",
                    "arena_version": "0.1.0",
                    "task_id": task_id,
                    "trial": trial,
                    "candidate_id": candidate_id,
                    "candidate_kind": "oneshot",
                    "reward": if passes { 1.0 } else { 0.0 },
                    "recall": if passes { 1.0 } else { 0.0 },
                    "false_positives": 0,
                    "findings": findings,
                    "artifacts": format!("artifacts/{candidate_id}/{task_id}"),
                });
                trials.push_str(&line.to_string());
                trials.push('\n');
            }
        }
    }
    let trials_jsonl = root.join("trials.jsonl");
    std::fs::write(&trials_jsonl, trials).expect("write trials.jsonl");

    let benchmark = "pass-k-compare-fixture";
    let run = |candidate_id: &str| {
        let spec_json = json!({
            "schema_version": "crucible.eval_spec.v1",
            "id": benchmark,
            "task": "cerberus-review-quality",
            "inputs": "Hermetic pass^k-shaped fixture.",
            "outputs": "Key recall and pass^2 task consistency.",
            "graders": {"graders": [{"id": "expected_key_match", "kind": "deterministic"}]},
            "aggregation": "proportion",
            "uncertainty": {"method": "wilson", "confidence": 0.95},
            "decision": "Prove pass^k paired noise-floor wiring without a sibling Daedalus checkout.",
            "runner": {
                "kind": "key_recall",
                "corpus": {
                    "source": "daedalus_trials",
                    "arena_dir": arena.display().to_string(),
                    "trials_jsonl": trials_jsonl.display().to_string(),
                    "candidate_id": candidate_id,
                    "tasks": task_ids,
                }
            }
        });
        let spec_path = root.join(format!("{candidate_id}.json"));
        std::fs::write(
            &spec_path,
            serde_json::to_string_pretty(&spec_json).unwrap(),
        )
        .expect("write candidate spec");
        let out_dir = root.join(format!("out-{candidate_id}"));
        let out = crucible()
            .arg("run")
            .arg(&spec_path)
            .arg("--out")
            .arg(&out_dir)
            .arg("--db")
            .arg(&db)
            .arg("--json")
            .output()
            .expect("crucible binary runs");
        assert!(
            out.status.success(),
            "run for {candidate_id} must persist; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: serde_json::Value =
            serde_json::from_slice(&out.stdout).expect("run --json emits JSON");
        assert!(
            v["evals"][0]["notes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|note| note.as_str().unwrap_or("").starts_with("pass^2:")),
            "{candidate_id} run must report a pass^2 score: {v}"
        );
    };
    for (candidate_id, _) in candidates {
        run(candidate_id);
    }

    let compare = |left: &str, right: &str| -> serde_json::Value {
        let out = crucible()
            .arg("runs")
            .arg("compare")
            .arg("--db")
            .arg(&db)
            .arg("--benchmark")
            .arg(benchmark)
            .arg("--left")
            .arg(left)
            .arg("--right")
            .arg(right)
            .arg("--alpha")
            .arg("0.05")
            .arg("--json")
            .output()
            .expect("crucible runs compare executes");
        assert!(
            out.status.success(),
            "runs compare {left}/{right} exits 0; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).expect("runs compare emits JSON")
    };

    let signal = compare("signal-left", "signal-right");
    assert_eq!(
        signal["comparison_kind"], "paired_mcnemar",
        "pass^k task rows must pair, not fall back to the unpaired delta: {signal}"
    );
    assert_eq!(signal["common_tasks"], 6);
    assert_eq!(signal["paired"]["b"], 0);
    assert_eq!(signal["paired"]["c"], 6);
    assert_eq!(signal["paired"]["verdict"], "signal");

    let noise = compare("noise-left", "noise-right");
    assert_eq!(noise["comparison_kind"], "paired_mcnemar");
    assert_eq!(noise["common_tasks"], 6);
    assert_eq!(noise["paired"]["b"], 3);
    assert_eq!(noise["paired"]["c"], 3);
    assert_eq!(noise["paired"]["verdict"], "inside_noise_floor");
}

/// A malformed `--since`/`--until` bound is a clean load error (exit 1, a
/// readable stderr message), not a panic/backtrace — `run_store::
/// parse_timestamp_bound` refuses garbage input before any query runs.
#[test]
fn runs_list_rejects_a_malformed_since_bound_cleanly() {
    let root = temp_root("run-db-bad-since");
    let db = root.join("runs.sqlite");
    let out_dir = root.join("out");
    let spec = fixture("specs/key-recall-fixture.json");

    let run = crucible()
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&out_dir)
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert!(
        run.status.success(),
        "seeding the ledger must succeed; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let bad_since = crucible()
        .arg("runs")
        .arg("list")
        .arg("--db")
        .arg(&db)
        .arg("--since")
        .arg("not-a-date")
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert_eq!(
        bad_since.status.code(),
        Some(1),
        "a malformed --since is a load error, not a panic: stderr={}",
        String::from_utf8_lossy(&bad_since.stderr)
    );
    let stderr = String::from_utf8_lossy(&bad_since.stderr);
    assert!(
        stderr.contains("not-a-date"),
        "error names the offending value: {stderr}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "no panic/backtrace, a clean error: {stderr}"
    );

    let bad_until = crucible()
        .arg("runs")
        .arg("list")
        .arg("--db")
        .arg(&db)
        .arg("--until")
        .arg("")
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert_eq!(
        bad_until.status.code(),
        Some(1),
        "an empty --until is likewise a clean load error: stderr={}",
        String::from_utf8_lossy(&bad_until.stderr)
    );
}

/// `crucible serve` is the local-first application face over the same run
/// ledger/spec validation contracts the CLI and MCP already expose. The UI
/// shell is static, but every table/detail view is backed by these JSON routes.
#[test]
fn serve_exposes_specs_runs_trends_and_run_detail_over_http() {
    let root = temp_root("serve-ui");
    let db = root.join("runs.sqlite");
    let out_dir = root.join("out");
    let spec = fixture("specs/key-recall-fixture.json");
    let bearer = "serve-pass";

    let run = crucible()
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&out_dir)
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert!(
        run.status.success(),
        "seed run must persist; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let (child, port) = spawn_serve(&db, &repo_fixture("evals"), Some(bearer));

    let shell = http_get(port, "/");
    assert!(shell.contains("Crucible"));
    assert!(
        shell.contains("Benchmark library") && shell.contains("Run setup"),
        "UI shell leads with benchmark/setup language: {shell}"
    );
    assert!(
        shell.contains("uncertainty range") && shell.contains("noise floor"),
        "UI explains statistical chips in operator language: {shell}"
    );
    assert!(
        shell.contains("ae-shell"),
        "UI shell composes the aesthetic app shell"
    );
    assert!(
        shell.contains("/api/specs") && shell.contains("/api/runs"),
        "shell is wired to the local API face"
    );

    let specs = http_get_json(port, "/api/specs");
    assert_eq!(specs["schema_version"], "crucible.ui.specs.v1");
    let specs_array = specs["specs"].as_array().expect("specs array");
    assert!(
        specs_array
            .iter()
            .any(|spec| spec["id"] == "cerberus-review-quality-v0"
                && spec["validation"]["valid"] == true
                && spec["confidence"] == 0.95),
        "spec library includes real eval specs with validation and confidence: {specs}"
    );
    let tracer = specs_array
        .iter()
        .find(|spec| spec["id"] == "tracer-exact-v1")
        .expect("tracer benchmark is listed");
    assert_eq!(tracer["object_label"], "benchmark");
    assert_eq!(tracer["task_count"], 37);
    assert_eq!(tracer["supports_controlled_comparison"], true);
    assert!(
        tracer["verifier_summary"]
            .as_str()
            .expect("verifier summary")
            .contains("Deterministic text verifier"),
        "benchmark card explains the verifier plainly: {tracer}"
    );
    assert_eq!(
        tracer["runner_defaults"]["tool_policy"],
        "No tools. The runner sends one text prompt to the model and grades the final text with deterministic verifiers."
    );
    let operator = specs_array
        .iter()
        .find(|spec| spec["id"] == "operator-micro-benchmark-v0")
        .expect("operator walkthrough benchmark is listed");
    assert_eq!(operator["task_count"], 5);
    assert_eq!(operator["supports_controlled_comparison"], true);
    let operator_summary = operator["plain_summary"]
        .as_str()
        .expect("plain summary")
        .to_ascii_lowercase();
    assert!(
        operator_summary.contains("five tiny exact-answer prompt tasks"),
        "operator benchmark explains itself plainly: {operator}"
    );

    let runs = http_get_json_auth(port, "/api/runs", bearer);
    assert_eq!(runs["schema_version"], "crucible.ui.runs.v1");
    let rows = runs["runs"].as_array().expect("runs array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["benchmark_id"], "key-recall-fixture");
    let run_id = rows[0]["run_id"].as_str().expect("run id");
    let trends = runs["trendlines"].as_array().expect("trendlines array");
    assert_eq!(trends.len(), 1);
    assert_eq!(trends[0]["benchmark_id"], "key-recall-fixture");
    assert_eq!(
        trends[0]["points"].as_array().expect("trend points").len(),
        1,
        "the runs API returns enough data for SVG sparklines"
    );

    let detail = http_get_json_auth(port, &format!("/api/runs/{run_id}"), bearer);
    assert_eq!(detail["schema_version"], "crucible.run_store.v1");
    assert_eq!(detail["run"]["run_id"], run_id);
    assert_eq!(detail["run"]["method"], "Wilson");
    assert_eq!(detail["eval_json"]["id"], "key-recall-fixture");

    stop_child(child);
}

/// crucible-904: the accept loop must not let one slow/stuck client starve
/// every other viewer. A connection that opens a TCP stream and never
/// finishes sending its request line blocks a single-threaded accept loop
/// forever (`HttpRequest::read` sits in `read_line` with no data and no EOF);
/// a genuinely concurrent server keeps answering other requests while that
/// connection just sits there.
#[test]
fn serve_answers_other_requests_while_one_client_never_finishes_its_request() {
    let root = temp_root("serve-concurrency");
    let db = root.join("runs.sqlite");
    let (child, port) = spawn_serve(&db, &repo_fixture("evals"), None);

    // Open a connection and deliberately send nothing — no request line, no
    // shutdown. The old single-threaded accept loop would still be inside
    // `handle_connection` for this connection when the second request below
    // arrives, and would never call `accept()` again to pick it up.
    let stuck = TcpStream::connect(("127.0.0.1", port)).expect("open a stuck connection");
    // Give a sequential accept loop time to actually pick this connection up
    // and block on it, so the reproduction isn't a timing coin flip.
    std::thread::sleep(Duration::from_millis(200));

    let started = std::time::Instant::now();
    let body = http_get(port, "/api/specs");
    let elapsed = started.elapsed();

    assert!(
        body.contains("schema_version"),
        "the concurrent request still gets a real response: {body}"
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "a concurrent request answered in {elapsed:?} while a stuck client held a connection \
         open; a single-threaded accept loop would block for the full 2s client read timeout \
         (or hang until the stuck connection's own timeout) instead"
    );

    drop(stuck);
    stop_child(child);
}

/// crucible-031: `crucible serve` must mount the same live writeback loop
/// `adjudication-panel --serve` runs as a separate process — not just
/// render-compose a read-only projection of an existing queue artifact. A
/// run carrying a `crucible.judgment_queue.v1` artifact must be openable
/// *and labelable* from inside the main serve shell, minting labels through
/// the exact same `apply_label` persistence path (`crucible.label.v1` JSON
/// array, sibling to the queue artifact) that the standalone `--serve` loop
/// and `adjudicate --apply` both read and write.
#[test]
fn serve_mounts_live_adjudication_writeback_for_a_queue_artifact() {
    let root = temp_root("serve-adjudication-live");
    let db = root.join("runs.sqlite");
    let out_dir = root.join("out");
    let bearer = "serve-pass";

    // `crucible run` with no spec/--eval executes all three built-in
    // receipts, including `recoverable-adjudication-queue`, which writes a
    // real `queue.json` + `panel/index.html` and persists both as artifacts
    // of its run row.
    let run = crucible()
        .arg("run")
        .arg("--out")
        .arg(&out_dir)
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert!(
        run.status.success(),
        "seed run must persist; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let (child, port) = spawn_serve(&db, &fixture("specs"), Some(bearer));

    let runs = http_get_json_auth(port, "/api/runs", bearer);
    let rows = runs["runs"].as_array().expect("runs array");
    let run_id = rows
        .iter()
        .find(|row| row["benchmark_id"] == "recoverable-adjudication-queue")
        .and_then(|row| row["run_id"].as_str())
        .expect("recoverable-adjudication-queue run is persisted")
        .to_string();

    let panel_html = http_get_auth(port, &format!("/adjudication/panel/{run_id}"), bearer);
    let label_path = format!("/adjudication/panel/{run_id}/label");
    // The panel embeds the run id percent-encoded (`:` -> `%3A`) in the
    // `fetch()` target; the raw form is what the test itself POSTs to below
    // (the server percent-decodes either way).
    let encoded_label_path = label_path.replace(':', "%3A");
    assert!(
        panel_html.contains("<script>") && panel_html.contains("fetch("),
        "the mounted panel must be the live-wired render, not the static \
         read-only projection: {panel_html}"
    );
    assert!(
        panel_html.contains(&encoded_label_path),
        "the live script must post verdicts to this run's own mounted label \
         route (not a shared /label), so multiple runs stay independently \
         labelable from one `crucible serve` process: {panel_html}"
    );

    let finding_id = panel_html
        .split("data-finding-id=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("panel renders at least one item with a finding id")
        .to_string();

    let label_body = serde_json::to_string(&json!({
        "finding_id": finding_id,
        "verdict": "keep",
        "in_scope": true,
        "latency_ms": 1234
    }))
    .expect("label request body");

    let unauth = http_post_json(port, &label_path, None, &label_body);
    assert!(
        unauth.starts_with("HTTP/1.1 401 Unauthorized"),
        "posting a label is a state-changing write and must require the \
         same bearer auth every other mutating serve route requires: {unauth}"
    );

    let response = http_post_json(port, &label_path, Some(bearer), &label_body);
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "authenticated label post must succeed: {response}"
    );
    let response_json: serde_json::Value =
        serde_json::from_str(&response_body(&response)).expect("label response is JSON");
    assert_eq!(response_json["ok"], true);
    assert_eq!(response_json["label"]["finding_id"], finding_id);
    assert_eq!(response_json["label"]["verdict"], "keep");

    // The mounted write path must land through the same persistence
    // `adjudication-panel --serve` uses: a `crucible.label.v1` array sibling
    // to the queue.json artifact, re-readable by `adjudicate --apply`.
    let labels_path = out_dir
        .join("recoverable-adjudication-queue")
        .join("labels.json");
    let persisted: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&labels_path).unwrap_or_else(|err| {
            panic!(
                "mounted route must persist to {}: {err}",
                labels_path.display()
            )
        }),
    )
    .expect("persisted labels are JSON");
    let persisted_array = persisted.as_array().expect("labels.json is an array");
    assert_eq!(persisted_array.len(), 1);
    assert_eq!(persisted_array[0]["finding_id"], finding_id);
    assert_eq!(persisted_array[0]["verdict"], "keep");

    // Re-fetching the panel must reflect the just-applied label, exactly as
    // the standalone `--serve` loop's `GET /` does after a `POST /label`.
    let refreshed = http_get_auth(port, &format!("/adjudication/panel/{run_id}"), bearer);
    assert!(
        refreshed.contains("Label: Keep"),
        "the mounted panel must reflect the applied label on next GET, not \
         a stale pre-label snapshot: {refreshed}"
    );

    stop_child(child);
}

#[test]
fn serve_requires_bearer_auth_for_run_reading_and_mutating_routes() {
    let root = temp_root("serve-auth");
    let db = root.join("runs.sqlite");
    let out_dir = root.join("out");
    let spec = fixture("specs/key-recall-fixture.json");
    let bearer = "serve-pass";

    let run = crucible()
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&out_dir)
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert!(
        run.status.success(),
        "seed run must persist; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let (child, port) = spawn_serve(&db, &fixture("specs"), Some(bearer));

    let specs = http_get_json(port, "/api/specs");
    assert_eq!(specs["schema_version"], "crucible.ui.specs.v1");

    let no_auth_runs = http_request(port, "GET", "/api/runs", &[], "");
    assert!(
        no_auth_runs.starts_with("HTTP/1.1 401 Unauthorized"),
        "run ledger listing requires auth: {no_auth_runs}"
    );
    let wrong_auth_runs = http_request(
        port,
        "GET",
        "/api/runs",
        &[("Authorization", "Bearer wrong")],
        "",
    );
    assert!(
        wrong_auth_runs.starts_with("HTTP/1.1 401 Unauthorized"),
        "wrong bearer token is rejected: {wrong_auth_runs}"
    );

    let runs = http_get_json_auth(port, "/api/runs", bearer);
    let run_id = runs["runs"][0]["run_id"].as_str().expect("seeded run id");
    let _detail = http_get_json_auth(port, &format!("/api/runs/{run_id}"), bearer);

    for path in [
        "/api/adjudication".to_string(),
        format!("/adjudication/panel/{run_id}"),
        format!("/artifacts/{run_id}/0"),
    ] {
        let response = http_request(port, "GET", &path, &[], "");
        assert!(
            response.starts_with("HTTP/1.1 401 Unauthorized"),
            "{path} requires auth before reading run-backed data: {response}"
        );
    }

    let run_body = serde_json::to_string(&json!({
        "spec": spec.display().to_string(),
        "out": root.join("no-auth-run").display().to_string()
    }))
    .expect("run request body");
    let no_auth_run = http_post_json(port, "/api/run", None, &run_body);
    assert!(
        no_auth_run.starts_with("HTTP/1.1 401 Unauthorized"),
        "state-changing run launch requires auth: {no_auth_run}"
    );

    stop_child(child);
}

#[test]
fn serve_confines_run_output_to_gitignored_runs_tree() {
    let root = temp_root("serve-out-confine");
    let db = root.join("runs.sqlite");
    let spec = fixture("specs/key-recall-fixture.json");
    let bearer = "serve-pass";
    let (child, port) = spawn_serve(&db, &fixture("specs"), Some(bearer));

    let escaped = root.join("escaped-output");
    let escaped_body = serde_json::to_string(&json!({
        "spec": spec.display().to_string(),
        "out": escaped.display().to_string()
    }))
    .expect("escaped run request body");
    let escaped_response = http_post_json(port, "/api/run", Some(bearer), &escaped_body);
    assert!(
        escaped_response.starts_with("HTTP/1.1 400 Bad Request"),
        "out outside runs/ is a client error: {escaped_response}"
    );
    assert!(
        !escaped.exists(),
        "rejected out path must not be created outside the runs tree"
    );

    let allowed = Path::new("runs").join("local").join(format!(
        "serve-out-confine-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let allowed_body = serde_json::to_string(&json!({
        "spec": spec.display().to_string(),
        "out": allowed.display().to_string()
    }))
    .expect("allowed run request body");
    let allowed_response = http_post_json(port, "/api/run", Some(bearer), &allowed_body);
    assert!(
        allowed_response.starts_with("HTTP/1.1 200 OK"),
        "runs/local out path remains usable: {allowed_response}"
    );
    assert!(
        allowed.join("run-report.json").exists(),
        "allowed out path receives the run report"
    );
    let _ = std::fs::remove_dir_all(&allowed);

    stop_child(child);
}

#[test]
fn run_prompt_benchmark_requires_openrouter_key_without_fallback() {
    let out_dir = temp_root("prompt-no-key");
    let out = crucible()
        .arg("run")
        .arg(repo_fixture("evals/prompt-smoke-v0.json"))
        .arg("--out")
        .arg(&out_dir)
        .arg("--json")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("crucible binary runs");

    assert_eq!(
        out.status.code(),
        Some(1),
        "missing model key is a load/runtime error, not usage"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("OPENROUTER_API_KEY") && stderr.contains("BYOK OpenRouter key"),
        "error names the missing key without a fallback: {stderr}"
    );
}

#[test]
fn run_prompt_benchmark_model_override_is_a_runtime_option_not_usage_error() {
    let out_dir = temp_root("prompt-model-override-no-key");
    let out = crucible()
        .arg("run")
        .arg(repo_fixture("evals/prompt-smoke-v0.json"))
        .arg("--out")
        .arg(&out_dir)
        .arg("--model")
        .arg("deepseek/deepseek-v4-flash")
        .arg("--json")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("crucible binary runs");

    assert_eq!(
        out.status.code(),
        Some(1),
        "--model should parse and reach the runner; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("OPENROUTER_API_KEY"),
        "runtime still names the missing BYOK key: {stderr}"
    );
}

#[test]
fn run_prompt_benchmark_models_fanout_is_a_runtime_option_not_usage_error() {
    let out_dir = temp_root("prompt-models-fanout-no-key");
    let out = crucible()
        .arg("run")
        .arg(repo_fixture("evals/prompt-smoke-v0.json"))
        .arg("--out")
        .arg(&out_dir)
        .arg("--models")
        .arg("deepseek/deepseek-v4-flash,z-ai/glm-5.2")
        .arg("--json")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("crucible binary runs");

    assert_eq!(
        out.status.code(),
        Some(1),
        "--models should parse and reach the runner; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("OPENROUTER_API_KEY"),
        "runtime still names the missing BYOK key: {stderr}"
    );
}

#[test]
fn run_prompt_benchmark_model_and_models_are_mutually_exclusive() {
    let out = crucible()
        .arg("run")
        .arg(repo_fixture("evals/prompt-smoke-v0.json"))
        .arg("--model")
        .arg("deepseek/deepseek-v4-flash")
        .arg("--models")
        .arg("z-ai/glm-5.2")
        .arg("--json")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("crucible binary runs");

    assert_eq!(
        out.status.code(),
        Some(1),
        "conflicting model flags are a Crucible runtime/config error"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "error should name the conflict: {stderr}"
    );
}

/// Backlog 017: the broadened deterministic grader library (`Regex`,
/// `CaseInsensitiveContains`) is real, not just a schema addition — a spec
/// declaring both variants validates clean and routes through the same
/// `prompt_benchmark` runner as `Exact`/`Contains` (reaching the BYOK
/// credential guard, proving the regex precompiled and dispatch worked,
/// without a live network call in the gate).
#[test]
fn regex_and_case_insensitive_contains_expectations_validate_and_dispatch() {
    let spec = repo_fixture("evals/prompt-regex-smoke-v0.json");

    let validate = crucible()
        .arg("validate")
        .arg(&spec)
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    let report: serde_json::Value =
        serde_json::from_slice(&validate.stdout).expect("validate --json emits JSON");
    assert_eq!(report["valid"], true, "{report}");
    assert_eq!(report["runnable"], true, "{report}");
    assert_eq!(report["errors"].as_array().unwrap().len(), 0);

    let out_dir = temp_root("prompt-regex-no-key");
    let out = crucible()
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&out_dir)
        .arg("--json")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("crucible binary runs");
    assert_eq!(
        out.status.code(),
        Some(1),
        "reaches the same BYOK credential guard as any other prompt_benchmark spec"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("OPENROUTER_API_KEY"),
        "the regex compiled fine and dispatch reached the client construction step: {stderr}"
    );
}

/// A spec declaring a `Regex` expectation whose pattern does not compile
/// refuses at `crucible validate` time — before any runnable corpus is
/// needed — and again at `crucible run` time before any model call.
#[test]
fn a_malformed_regex_expectation_refuses_at_validate_and_run_time() {
    let root = temp_root("prompt-regex-malformed");
    let spec_text = std::fs::read_to_string(repo_fixture("evals/prompt-regex-smoke-v0.json"))
        .expect("read prompt-regex-smoke-v0.json");
    let mut spec: serde_json::Value = serde_json::from_str(&spec_text).unwrap();
    spec["runner"]["corpus"]["tasks"][0]["expectation"]["pattern"] = json!("(unclosed");
    let spec_path = root.join("malformed-regex.json");
    std::fs::write(&spec_path, serde_json::to_string_pretty(&spec).unwrap())
        .expect("write malformed-regex spec");

    let validate = crucible()
        .arg("validate")
        .arg(&spec_path)
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    let report: serde_json::Value =
        serde_json::from_slice(&validate.stdout).expect("validate --json emits JSON");
    assert_eq!(report["valid"], false, "{report}");
    assert_eq!(report["runnable"], false, "{report}");
    let errors = report["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0]["message"]
            .as_str()
            .unwrap()
            .contains("phone-number-format"),
        "{report}"
    );

    let out_dir = root.join("out");
    let run = crucible()
        .arg("run")
        .arg(&spec_path)
        .arg("--out")
        .arg(&out_dir)
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert_eq!(
        run.status.code(),
        Some(1),
        "run must also refuse a malformed regex, independent of --json validate"
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains("(unclosed"),
        "run's error names the offending pattern: {stderr}"
    );
    assert!(
        !stderr.contains("OPENROUTER_API_KEY"),
        "refuses on the malformed pattern before ever reaching the credential check: {stderr}"
    );
}

/// The agentic judge runner (backlog 012) parses its declared spec, validates
/// the `Agentic` grader declaration, and reaches the same BYOK credential
/// guard as the prompt benchmark runner — proving the CLI dispatch wire-up
/// end to end without a live model call.
#[test]
fn run_agentic_judge_requires_openrouter_key_without_fallback() {
    let out_dir = temp_root("judge-no-key");
    let out = crucible()
        .arg("run")
        .arg(repo_fixture("evals/agentic-judge-smoke-v0.json"))
        .arg("--out")
        .arg(&out_dir)
        .arg("--json")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("crucible binary runs");

    assert_eq!(
        out.status.code(),
        Some(1),
        "missing model key is a load/runtime error, not usage"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("OPENROUTER_API_KEY") && stderr.contains("BYOK OpenRouter key"),
        "error names the missing key without a fallback: {stderr}"
    );
}

/// `crucible runs judge-status` (backlog 029) answers "is this judge licensed"
/// by licence key without any run ever having existed — reads as
/// locked/unlicensed (`null`), not an error, since an unmeasured identity is a
/// normal, expected state for a licence key nobody has run yet.
#[test]
fn runs_judge_status_reports_no_licence_for_an_unmeasured_key() {
    let out_dir = temp_root("judge-status-empty");
    let db = out_dir.join("runs.sqlite");

    let out = crucible()
        .arg("runs")
        .arg("judge-status")
        .arg("--licence-key")
        .arg("judge-licence:v1:no/such-judge:hash-a:hash-b")
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .output()
        .expect("crucible runs judge-status executes");
    assert!(
        out.status.success(),
        "an unmeasured licence key is a normal query result, not a usage error; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let status: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("judge-status emits JSON");
    assert!(
        status.is_null(),
        "no run has measured this key, so the standing licence is null: {status}"
    );
}

/// MCP exposes the same declared-spec runner as the CLI: initialize the stdio
/// server, list tools, call `crucible_run`, and get the scored run report plus
/// the on-disk evidence packet.
#[test]
fn mcp_crucible_run_executes_declared_spec() {
    let out_dir = temp_root("mcp-run");
    let db = out_dir.join("runs.sqlite");
    let db_arg = db.display().to_string();
    let spec = fixture("specs/key-recall-fixture.json");
    let mut child = crucible()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn crucible mcp");
    let mut stdin = child.stdin.take().expect("MCP stdin");
    let stdout = child.stdout.take().expect("MCP stdout");
    let mut stdout = BufReader::new(stdout);

    write_jsonrpc(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2025-11-25" }
        }),
    );
    let initialized = read_jsonrpc(&mut stdout);
    assert!(
        initialized.get("error").is_none(),
        "initialize succeeds: {initialized}"
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], "crucible");

    write_jsonrpc(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );
    let tools = read_jsonrpc(&mut stdout);
    assert!(tools.get("error").is_none(), "tools/list succeeds: {tools}");
    let tool_names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert!(
        tool_names.contains(&"crucible_run"),
        "MCP exposes the eval runner as an agent-callable tool: {tool_names:?}"
    );

    write_jsonrpc(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crucible_run",
                "arguments": {
                    "spec": spec,
                    "out": out_dir,
                    "db": db_arg.clone()
                }
            }
        }),
    );
    let call = read_jsonrpc(&mut stdout);
    assert!(call.get("error").is_none(), "tools/call succeeds: {call}");
    let report = &call["result"]["structuredContent"]["report"];
    assert_eq!(report["schema_version"], "crucible.run_report.v1");
    assert_eq!(report["evals"][0]["id"], "key-recall-fixture");
    assert_eq!(report["evals"][0]["score"]["successes"], 1);
    assert_eq!(report["evals"][0]["score"]["n"], 2);
    assert_eq!(report["evals"][0]["score"]["method"], "Wilson");
    assert!(call["result"]["content"][0]["text"]
        .as_str()
        .expect("text result")
        .contains("\"schema_version\": \"crucible.run_report.v1\""));

    let run_report_path = call["result"]["structuredContent"]["run_report"]
        .as_str()
        .expect("run_report path");
    assert!(
        Path::new(run_report_path).exists(),
        "MCP call writes run-report.json evidence"
    );
    assert_eq!(
        call["result"]["structuredContent"]["run_store"]["run_records"], 1,
        "MCP run also persists into the run ledger"
    );

    write_jsonrpc(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "crucible_runs_list",
                "arguments": {
                    "db": db_arg,
                    "benchmark": "key-recall-fixture"
                }
            }
        }),
    );
    let listed = read_jsonrpc(&mut stdout);
    assert!(
        listed.get("error").is_none(),
        "MCP runs list succeeds: {listed}"
    );
    assert_eq!(
        listed["result"]["structuredContent"]["runs"][0]["benchmark_id"],
        "key-recall-fixture"
    );

    write_jsonrpc(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "shutdown" }),
    );
    let shutdown = read_jsonrpc(&mut stdout);
    assert!(
        shutdown.get("error").is_none(),
        "shutdown succeeds: {shutdown}"
    );
    drop(stdin);
    let status = child.wait().expect("MCP process exits");
    assert!(status.success(), "MCP process exits cleanly");
}

/// `crucible_validate` over MCP (backlog 014): an agent checks a spec before
/// spending a `crucible_run` call on it.
#[test]
fn mcp_crucible_validate_confirms_a_real_fixture_spec_is_valid_and_runnable() {
    let spec = fixture("specs/key-recall-fixture.json");
    let mut child = crucible()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn crucible mcp");
    let mut stdin = child.stdin.take().expect("MCP stdin");
    let stdout = child.stdout.take().expect("MCP stdout");
    let mut stdout = BufReader::new(stdout);

    write_jsonrpc(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2025-11-25" }
        }),
    );
    read_jsonrpc(&mut stdout);

    write_jsonrpc(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crucible_validate",
                "arguments": { "spec": spec }
            }
        }),
    );
    let call = read_jsonrpc(&mut stdout);
    assert!(
        call.get("error").is_none(),
        "crucible_validate tool call succeeds: {call}"
    );
    let report = &call["result"]["structuredContent"];
    assert_eq!(report["schema_version"], "crucible.validate_report.v1");
    assert_eq!(
        report["valid"], true,
        "the real key-recall fixture spec is a valid, honest spec: {report}"
    );
    assert_eq!(report["runnable"], true);
    assert_eq!(report["errors"].as_array().unwrap().len(), 0);

    write_jsonrpc(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" }),
    );
    read_jsonrpc(&mut stdout);
    drop(stdin);
    let status = child.wait().expect("MCP process exits");
    assert!(status.success(), "MCP process exits cleanly");
}

/// Backlog 021: `crucible_grade`/`crucible_adjudicate`/`crucible_export` over
/// MCP are the same computations the CLI subcommands run, not a second
/// implementation — this drives all three end to end over one real stdio
/// JSON-RPC session, chaining adjudicate's queue into export's --labels the
/// way an agent lane actually would.
#[test]
fn mcp_exposes_grade_adjudicate_and_export_as_the_same_cli_computations() {
    let artifact = fixture("cerberus-artifact.json");
    let key = fixture("key.json");
    let mut child = crucible()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn crucible mcp");
    let mut stdin = child.stdin.take().expect("MCP stdin");
    let stdout = child.stdout.take().expect("MCP stdout");
    let mut stdout = BufReader::new(stdout);

    write_jsonrpc(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2025-11-25" }
        }),
    );
    read_jsonrpc(&mut stdout);

    write_jsonrpc(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crucible_grade",
                "arguments": { "artifact": artifact, "key": key }
            }
        }),
    );
    let graded = read_jsonrpc(&mut stdout);
    assert!(
        graded.get("error").is_none(),
        "crucible_grade tool call succeeds: {graded}"
    );
    let grade_report = &graded["result"]["structuredContent"];
    assert_eq!(grade_report["schema_version"], "crucible.grade_report.v1");
    assert_eq!(grade_report["matched"], 1);
    assert_eq!(grade_report["missed"], 1);

    write_jsonrpc(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crucible_adjudicate",
                "arguments": { "artifact": artifact, "key": key }
            }
        }),
    );
    let adjudicated = read_jsonrpc(&mut stdout);
    assert!(
        adjudicated.get("error").is_none(),
        "crucible_adjudicate tool call succeeds: {adjudicated}"
    );
    let queue = &adjudicated["result"]["structuredContent"];
    assert_eq!(queue["summary"]["matched"], 1);

    let export_labels = fixture("export-queue.json");
    let export_out = temp_root("mcp-export");
    let export_key = fixture("export-original-key.json");
    write_jsonrpc(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "crucible_export",
                "arguments": {
                    "labels": export_labels,
                    "out": export_out,
                    "arena": "pr-review-v0",
                    "task": "py-file-cache",
                    "base_version": "0.2.0",
                    "date": "2026-06-29",
                    "key": export_key
                }
            }
        }),
    );
    let exported = read_jsonrpc(&mut stdout);
    assert!(
        exported.get("error").is_none(),
        "crucible_export tool call succeeds: {exported}"
    );
    let export_report = &exported["result"]["structuredContent"];
    assert_eq!(export_report["schema_version"], "crucible.export_report.v1");
    assert_eq!(export_report["arena"], "pr-review-v0");
    assert_eq!(export_report["adjudications"], 3);
    assert_eq!(export_report["accepts"], 1);
    assert!(
        export_report["oracle_key"].is_string(),
        "a --key was given, so the report names the written oracle key: {export_report}"
    );
    assert!(
        std::fs::metadata(export_out.join("adjudications.md")).is_ok(),
        "the MCP tool actually wrote adjudications.md, not just a structured report"
    );

    write_jsonrpc(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "shutdown" }),
    );
    read_jsonrpc(&mut stdout);
    drop(stdin);
    let status = child.wait().expect("MCP process exits");
    assert!(status.success(), "MCP process exits cleanly");
}

/// The same declared runner must also consume fresh Cerberus producer handoffs:
/// a `ReviewArtifact` bound to a `ReviewReceiptBundle.v1`, then scored by
/// Crucible against the Harbor key.
#[test]
fn run_declared_spec_grades_cerberus_receipt_bundle_artifacts() {
    let out_dir = temp_root("run-cerberus-spec");
    let db = out_dir.join("runs.sqlite");
    let spec = fixture("specs/cerberus-receipt-fixture.json");
    let out = crucible()
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&out_dir)
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "declared Cerberus receipt spec run must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("run <spec> --json emits JSON");
    let eval = &v["evals"][0];
    assert_eq!(eval["id"], "cerberus-receipt-fixture");
    assert_eq!(eval["score"]["metric"], "pr_review_key_recall");
    assert_eq!(eval["score"]["successes"], 1);
    assert_eq!(eval["score"]["n"], 2);
    assert_eq!(eval["score"]["method"], "Wilson");

    let evidence_path = out_dir.join("task-results.json");
    let evidence: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(evidence_path).expect("read evidence"))
            .expect("evidence is JSON");
    assert_eq!(evidence["corpus"]["source"], "cerberus_receipt_bundles");
    assert_eq!(evidence["corpus"]["candidate_id"], "cerberus-fixture");
    assert_eq!(evidence["corpus"]["tasks"][0]["harness"], "fixture");
    assert_eq!(
        evidence["corpus"]["tasks"][0]["validation_status"],
        "passed"
    );
    assert_eq!(evidence["tasks"][0]["receipt_harness"], "fixture");
    assert_eq!(evidence["tasks"][0]["matched"], 1);
    assert_eq!(evidence["tasks"][0]["expected_defects"], 2);
}

/// Backlog 014 child 5: a cold-agent-authored spec (fresh content, not a copy
/// of `cerberus-receipt-fixture.json`'s exact shape — see
/// `tests/fixtures/specs/cold-agent-smoke-v0.json` and its three referenced
/// fixture files) validates and runs hermetically end to end: no
/// `OPENROUTER_API_KEY`, no sibling checkout, no network. `key_recall` over a
/// `cerberus_receipt_bundles` corpus is the genuinely hermetic runner family
/// (unlike `prompt_benchmark`, which always makes a live OpenRouter call) —
/// SKILL.md documents the corpus fields but not the internal shape its
/// referenced `artifact`/`receipt_bundle`/`expected` files must carry (e.g.
/// `receipt_bundle.validation.status` must be `"passed"`, `artifact_uri` must
/// match the declared `task.artifact` string); that gap is exactly the signal
/// this child was meant to surface.
#[test]
fn cold_agent_authored_spec_validates_and_runs_hermetically() {
    let spec = fixture("specs/cold-agent-smoke-v0.json");

    let validate = crucible()
        .arg("validate")
        .arg(&spec)
        .arg("--json")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("crucible binary runs");
    assert_eq!(validate.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_slice(&validate.stdout).expect("validate --json emits JSON");
    assert_eq!(
        report["valid"], true,
        "cold-agent-authored spec must be valid: {report}"
    );
    assert_eq!(report["runnable"], true);
    assert_eq!(report["errors"].as_array().unwrap().len(), 0);

    let out_dir = temp_root("cold-agent-smoke");
    let run = crucible()
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&out_dir)
        .arg("--json")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("crucible binary runs");
    assert_eq!(
        run.status.code(),
        Some(0),
        "cold-agent-authored spec must run hermetically with no OPENROUTER_API_KEY set; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&run.stdout).expect("run --json emits JSON");
    let eval = &report["evals"][0];
    assert_eq!(eval["id"], "cold-agent-smoke-v0");
    assert_eq!(eval["score"]["metric"], "pr_review_key_recall");
    assert_eq!(
        eval["score"]["successes"], 1,
        "the one seeded matching defect"
    );
    assert_eq!(
        eval["score"]["n"], 2,
        "one matched, one deliberately missed"
    );
}

/// The standalone panel command renders an existing judgment queue artifact into
/// a phone-first static HTML panel plus the copied queue model.
#[test]
fn adjudication_panel_renders_existing_queue_artifact() {
    let out_dir = temp_root("panel");
    let out = crucible()
        .arg("adjudication-panel")
        .arg("--queue")
        .arg(fixture("export-queue.json"))
        .arg("--out")
        .arg(&out_dir)
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "panel must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let html_path = out_dir.join("index.html");
    let queue_path = out_dir.join("queue.json");
    assert!(html_path.exists(), "panel HTML written");
    assert!(queue_path.exists(), "queue model copied");

    let html = std::fs::read_to_string(html_path).expect("read panel HTML");
    for marker in [
        "name=\"viewport\"",
        "Adjudication panel",
        "F3",
        "cache.py:23",
        "Keep",
        "Nit",
        "Wrong",
        "Noise",
    ] {
        assert!(html.contains(marker), "missing marker {marker:?}: {html}");
    }
}

/// Stable exit codes so Cerberus/Daedalus can branch headlessly: `0` success,
/// `1` a load/parse failure, `2` a usage error. This pins all three.
#[test]
fn exit_codes_are_stable_across_success_load_error_and_usage_error() {
    let ok = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg(fixture("cerberus-artifact.json"))
        .arg("--key")
        .arg(fixture("key.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert_eq!(ok.status.code(), Some(0), "success is exit 0");

    let load_error = crucible()
        .arg("grade")
        .arg("--artifact")
        .arg("/no/such/crucible/artifact.json")
        .arg("--key")
        .arg(fixture("key.json"))
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert_eq!(
        load_error.status.code(),
        Some(1),
        "a missing input is a load error, exit 1"
    );

    // No subcommand at all: clap usage error, exit 2.
    let usage_error = crucible().output().expect("crucible binary runs");
    assert_eq!(
        usage_error.status.code(),
        Some(2),
        "a usage error is exit 2"
    );
}

/// `crucible validate` over the real committed specs (backlog 014): every
/// fixture spec must be honest about what the runner actually enforces.
#[test]
fn validate_reports_the_real_flagship_specs_as_valid_with_expected_warnings() {
    for (spec, expect_warnings) in [
        (repo_fixture("evals/agentic-judge-smoke-v0.json"), false),
        (
            repo_fixture("evals/operator-micro-benchmark-v0.json"),
            false,
        ),
        (repo_fixture("evals/prompt-smoke-v0.json"), false),
        (repo_fixture("evals/tracer-exact-v1.json"), false),
        (repo_fixture("evals/pr-review-key-recall-v0.json"), true),
        (repo_fixture("evals/cerberus-review-quality-v0.json"), true),
    ] {
        let out = crucible()
            .arg("validate")
            .arg(&spec)
            .arg("--json")
            .output()
            .expect("crucible binary runs");
        assert_eq!(
            out.status.code(),
            Some(0),
            "validate always exits 0 for a spec that loads; verdict is in the body: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: serde_json::Value =
            serde_json::from_slice(&out.stdout).expect("validate --json emits JSON");
        assert_eq!(report["schema_version"], "crucible.validate_report.v1");
        assert_eq!(
            report["valid"], true,
            "{spec:?} should be a valid, honest spec: {report}"
        );
        assert_eq!(
            report["runnable"], true,
            "{spec:?} declares a runner and passes every preflight check: {report}"
        );
        assert_eq!(report["errors"].as_array().unwrap().len(), 0);
        let has_warnings = !report["warnings"].as_array().unwrap().is_empty();
        assert_eq!(
            has_warnings, expect_warnings,
            "{spec:?} warning expectation: {report}"
        );
    }
}

/// A negative fixture: a spec declaring `uncertainty.confidence` the runner
/// does not honor must fail validation with a named field and an unambiguous
/// message, without needing any runnable corpus (no sibling checkout, no
/// trials file, no API key) — the whole point of validating before running.
#[test]
fn validate_refuses_a_spec_that_declares_an_unhonored_confidence() {
    let root = temp_root("validate-bad-confidence");
    let spec_text = std::fs::read_to_string(repo_fixture("evals/prompt-smoke-v0.json"))
        .expect("read prompt-smoke-v0.json");
    let mut spec: serde_json::Value = serde_json::from_str(&spec_text).unwrap();
    spec["uncertainty"]["confidence"] = json!(0.99);
    let spec_path = root.join("bad-confidence.json");
    std::fs::write(&spec_path, serde_json::to_string_pretty(&spec).unwrap())
        .expect("write bad-confidence spec");

    let out = crucible()
        .arg("validate")
        .arg(&spec_path)
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert_eq!(out.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("validate --json emits JSON");
    assert_eq!(report["valid"], false);
    assert_eq!(report["runnable"], false);
    let errors = report["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["field"], "runner");
    assert!(
        errors[0]["message"].as_str().unwrap().contains("0.99"),
        "{report}"
    );
}

/// `validate` distinguishes a load error (unknown schema/malformed JSON —
/// exit 1) from a validation finding (exit 0, verdict in the body) — the
/// same exit-code discipline every other subcommand uses.
#[test]
fn validate_exits_1_on_a_load_error_not_0_with_a_finding() {
    let root = temp_root("validate-load-error");
    let spec_path = root.join("not-json.json");
    std::fs::write(&spec_path, "not valid json").expect("write malformed spec");

    let out = crucible()
        .arg("validate")
        .arg(&spec_path)
        .arg("--json")
        .output()
        .expect("crucible binary runs");
    assert_eq!(
        out.status.code(),
        Some(1),
        "a spec that fails to parse is a load error, not a validation finding"
    );
}
