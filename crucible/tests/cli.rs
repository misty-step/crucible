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

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

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
    let out = crucible()
        .arg("run")
        .arg("--out")
        .arg(&out_dir)
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
    let spec = fixture("specs/key-recall-fixture.json");
    let out = crucible()
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&out_dir)
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

/// MCP exposes the same declared-spec runner as the CLI: initialize the stdio
/// server, list tools, call `crucible_run`, and get the scored run report plus
/// the on-disk evidence packet.
#[test]
fn mcp_crucible_run_executes_declared_spec() {
    let out_dir = temp_root("mcp-run");
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
    assert_eq!(
        tools["result"]["tools"][0]["name"], "crucible_run",
        "MCP exposes the eval runner as an agent-callable tool"
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
                    "out": out_dir
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

    write_jsonrpc(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 4, "method": "shutdown" }),
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

/// The same declared runner must also consume fresh Cerberus producer handoffs:
/// a `ReviewArtifact` bound to a `ReviewReceiptBundle.v1`, then scored by
/// Crucible against the Harbor key.
#[test]
fn run_declared_spec_grades_cerberus_receipt_bundle_artifacts() {
    let out_dir = temp_root("run-cerberus-spec");
    let spec = fixture("specs/cerberus-receipt-fixture.json");
    let out = crucible()
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&out_dir)
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
