//! End-to-end tests for `crucible author`, driving the built binary as a
//! subprocess (matching `tests/cli.rs`'s no-`assert_cmd` convention — Cargo
//! hands integration tests `CARGO_BIN_EXE_crucible`).
//!
//! These prove the CLI-shaped acceptance for crucible-942: a spec assembled
//! through flags or `--interactive` is a real `evals/*.json`-shaped file that
//! `crucible validate`/`crucible run` read back exactly like a hand-written
//! one, an invalid flag combination refuses with a clear error and writes
//! nothing, and a spec that fails the save-gate validation is refused and
//! leaves no file behind.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn crucible() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crucible"))
}

fn temp_root(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "crucible-author-cli-{}-{tag}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp root");
    dir
}

#[test]
fn author_flags_produce_a_spec_that_round_trips_through_validate() {
    let dir = temp_root("prompt-benchmark");
    let out = dir.join("prompt-smoke-authored.json");

    let output = crucible()
        .args([
            "author",
            "--task-family",
            "prompt-smoke",
            "--runner-kind",
            "prompt_benchmark",
            "--prompt-model",
            "openrouter/auto",
            "--prompt-system-prompt",
            "Answer exactly.",
            "--prompt-task-id",
            "marker-echo",
            "--prompt-task-prompt",
            "Reply with crucible-smoke",
            "--prompt-expectation-kind",
            "contains",
            "--prompt-expectation-value",
            "crucible-smoke",
            "--out",
        ])
        .arg(&out)
        .arg("--json")
        .output()
        .expect("run crucible author");

    assert!(
        output.status.success(),
        "author failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.exists(), "expected {} to be written", out.display());

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("author --json emits a JSON object");
    assert_eq!(report["schema_version"], "crucible.author_report.v1");
    assert_eq!(report["written"], true);
    assert_eq!(report["validate"]["valid"], true);
    assert_eq!(report["validate"]["runnable"], true);

    // The same `crucible validate` a hand-authored spec would go through —
    // proves this is a real evals/*.json-shaped file, not a parallel format.
    let validate_output = crucible()
        .arg("validate")
        .arg(&out)
        .arg("--json")
        .output()
        .expect("run crucible validate");
    assert!(validate_output.status.success());
    let validate_report: serde_json::Value = serde_json::from_slice(&validate_output.stdout)
        .expect("validate --json emits a JSON object");
    assert_eq!(validate_report["valid"], true);
    assert_eq!(validate_report["runnable"], true);
}

#[test]
fn author_key_recall_flags_produce_a_valid_spec() {
    let dir = temp_root("key-recall");
    let out = dir.join("key-recall-authored.json");

    let output = crucible()
        .args([
            "author",
            "--task-family",
            "pr-review-key-recall",
            "--runner-kind",
            "key_recall",
            "--key-recall-arena-dir",
            "../../daedalus/arenas/pr-review-v0",
            "--key-recall-trials-jsonl",
            "../../daedalus/runs/freeze/trials.jsonl",
            "--key-recall-candidate-id",
            "probe-oneshot",
            "--out",
        ])
        .arg(&out)
        .arg("--json")
        .output()
        .expect("run crucible author");

    assert!(
        output.status.success(),
        "author failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.exists());

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("author --json emits a JSON object");
    assert_eq!(report["written"], true);
    assert_eq!(report["validate"]["valid"], true);
    assert_eq!(report["validate"]["runnable"], true);

    let spec: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&out).unwrap()).expect("written file is JSON");
    assert_eq!(spec["runner"]["kind"], "key_recall");
    assert_eq!(spec["runner"]["corpus"]["source"], "daedalus_trials");
}

#[test]
fn author_missing_required_flags_refuses_with_no_file_written() {
    let dir = temp_root("missing-flags");
    let out = dir.join("should-not-exist.json");

    let output = crucible()
        .args(["author", "--task-family", "prompt-smoke", "--out"])
        .arg(&out)
        .output()
        .expect("run crucible author");

    assert!(
        !output.status.success(),
        "author should fail when --runner-kind is missing"
    );
    assert!(
        !out.exists(),
        "no file should be written on an invalid flag combination"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--runner-kind"), "stderr: {stderr}");
}

#[test]
fn author_refuses_an_invalid_grader_mix_and_leaves_no_file() {
    let dir = temp_root("invalid-grader-mix");
    let out = dir.join("bad-grader-mix.json");

    let output = crucible()
        .args([
            "author",
            "--task-family",
            "prompt-smoke",
            "--runner-kind",
            "prompt_benchmark",
            "--prompt-model",
            "openrouter/auto",
            "--prompt-system-prompt",
            "Answer exactly.",
            "--prompt-task-id",
            "marker-echo",
            "--prompt-task-prompt",
            "Reply with crucible-smoke",
            "--prompt-expectation-kind",
            "contains",
            "--prompt-expectation-value",
            "crucible-smoke",
            // prompt_benchmark requires a deterministic grader; naming only
            // a human grader must be refused by the save-gate validation,
            // not silently rewritten.
            "--grader",
            "operator:human",
            "--out",
        ])
        .arg(&out)
        .arg("--json")
        .output()
        .expect("run crucible author");

    assert!(
        !output.status.success(),
        "author should refuse to save an invalid spec"
    );
    assert!(
        !out.exists(),
        "no file should be written for an invalid spec"
    );

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("author --json emits a JSON object");
    assert_eq!(report["written"], false);
    assert_eq!(report["validate"]["valid"], false);
    assert!(!report["validate"]["errors"]
        .as_array()
        .expect("errors array")
        .is_empty());

    // No scratch file left behind either.
    let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert!(
        leftovers.is_empty(),
        "scratch file must be cleaned up: {leftovers:?}"
    );
}

#[test]
fn author_refuses_to_overwrite_an_existing_spec_without_force() {
    let dir = temp_root("no-clobber");
    let out = dir.join("existing.json");
    std::fs::write(&out, "not a real spec, just a sentinel").unwrap();

    let output = crucible()
        .args([
            "author",
            "--task-family",
            "prompt-smoke",
            "--runner-kind",
            "prompt_benchmark",
            "--prompt-model",
            "openrouter/auto",
            "--prompt-system-prompt",
            "Answer exactly.",
            "--prompt-task-id",
            "marker-echo",
            "--prompt-task-prompt",
            "Reply with crucible-smoke",
            "--prompt-expectation-kind",
            "contains",
            "--prompt-expectation-value",
            "crucible-smoke",
            "--out",
        ])
        .arg(&out)
        .output()
        .expect("run crucible author");

    assert!(!output.status.success());
    let contents = std::fs::read_to_string(&out).unwrap();
    assert_eq!(
        contents, "not a real spec, just a sentinel",
        "existing file must be untouched without --force"
    );
}

#[test]
fn author_interactive_drives_a_prompt_benchmark_spec_via_stdin() {
    let dir = temp_root("interactive");
    let out = dir.join("interactive-spec.json");

    let mut child = crucible()
        .arg("author")
        .arg("--interactive")
        .arg("--out")
        .arg(&out)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn crucible author --interactive");

    let script = "\n\
        \n\
        code-review\n\
        \n\
        \n\
        \n\
        \n\
        prompt_benchmark\n\
        openrouter/auto\n\
        Answer exactly.\n\
        \n\
        \n\
        \n\
        marker-echo\n\
        Reply with crucible-smoke\n\
        \n\
        contains\n\
        crucible-smoke\n\
        \n";
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(script.as_bytes())
        .expect("write scripted stdin");

    let output = child.wait_with_output().expect("wait for author process");
    assert!(
        output.status.success(),
        "interactive author failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.exists(), "expected {} to be written", out.display());

    let spec: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&out).unwrap()).expect("written file is JSON");
    assert_eq!(spec["task"], "code-review");
    assert!(spec.get("context").is_none());
    assert_eq!(spec["runner"]["kind"], "prompt_benchmark");

    let validate_output = crucible()
        .arg("validate")
        .arg(&out)
        .arg("--json")
        .output()
        .expect("run crucible validate");
    assert!(validate_output.status.success());
    let validate_report: serde_json::Value = serde_json::from_slice(&validate_output.stdout)
        .expect("validate --json emits a JSON object");
    assert_eq!(validate_report["valid"], true);
    assert_eq!(validate_report["runnable"], true);
}

#[test]
fn authored_spec_is_visible_to_crucible_serve_benchmarks_list() {
    let dir = temp_root("serve-visibility");
    let specs_dir = dir.join("evals");
    std::fs::create_dir_all(&specs_dir).unwrap();
    let out = specs_dir.join("author-serve-smoke.json");

    let author_output = crucible()
        .args([
            "author",
            "--id",
            "author-serve-smoke-v0",
            "--task-family",
            "prompt-smoke",
            "--runner-kind",
            "prompt_benchmark",
            "--prompt-model",
            "openrouter/auto",
            "--prompt-system-prompt",
            "Answer exactly.",
            "--prompt-task-id",
            "marker-echo",
            "--prompt-task-prompt",
            "Reply with crucible-smoke",
            "--prompt-expectation-kind",
            "contains",
            "--prompt-expectation-value",
            "crucible-smoke",
            "--out",
        ])
        .arg(&out)
        .output()
        .expect("run crucible author");
    assert!(
        author_output.status.success(),
        "author failed: stdout={} stderr={}",
        String::from_utf8_lossy(&author_output.stdout),
        String::from_utf8_lossy(&author_output.stderr)
    );

    let db = dir.join("crucible-runs.sqlite");
    let mut serve = crucible()
        .arg("serve")
        .arg("--db")
        .arg(&db)
        .arg("--specs")
        .arg(&specs_dir)
        .arg("--port")
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn crucible serve");

    let Some(port) = read_bound_port(&mut serve) else {
        let _ = serve.wait();
        return;
    };
    let body = http_get(port, "/api/specs");
    let response: serde_json::Value = serde_json::from_str(&body).expect("specs response is JSON");
    let ids: Vec<String> = response["specs"]
        .as_array()
        .expect("specs response has a specs array")
        .iter()
        .map(|s| {
            s.get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert!(
        ids.iter().any(|id| id == "author-serve-smoke-v0"),
        "authored spec not visible in /api/specs: {ids:?}"
    );

    let _ = serve.kill();
    let _ = serve.wait();
}

fn read_bound_port(child: &mut std::process::Child) -> Option<u16> {
    use std::io::{BufRead, BufReader, Read};
    let stdout = child.stdout.take().expect("serve stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    for _ in 0..20 {
        line.clear();
        let n = reader.read_line(&mut line).expect("read serve stdout");
        if n == 0 {
            break;
        }
        if let Some(port_str) = line.trim().strip_prefix("crucible serve listening on ") {
            if let Ok(port) = port_str
                .trim_start_matches("127.0.0.1:")
                .trim()
                .parse::<u16>()
            {
                return Some(port);
            }
        }
        if let Some(idx) = line.find("127.0.0.1:") {
            let rest = &line[idx + "127.0.0.1:".len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(port) = digits.parse::<u16>() {
                return Some(port);
            }
        }
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    if stderr.contains("Operation not permitted") {
        eprintln!("skipping serve visibility test: loopback bind refused by OS: {stderr}");
        return None;
    }
    panic!("could not read bound port from crucible serve stdout: {line:?}; stderr={stderr}");
}

fn http_get(port: u16, path: &str) -> String {
    use std::io::Read;
    use std::net::{Shutdown, TcpStream};
    use std::time::Duration;

    let mut stream = None;
    for _ in 0..50 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            stream = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut stream = stream.expect("connect to crucible serve");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set read timeout");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .expect("write HTTP request");
    stream
        .shutdown(Shutdown::Write)
        .expect("finish HTTP request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read HTTP response");
    assert!(
        response.starts_with("HTTP/1.1 200 OK"),
        "response: {response}"
    );
    response
        .split("\r\n\r\n")
        .nth(1)
        .expect("HTTP response has a body")
        .to_string()
}

#[allow(dead_code)]
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
