//! End-to-end tests for `crucible doctor` (crucible-911), driving the real
//! compiled binary as a subprocess exactly like `tests/cli.rs` does.
//!
//! `check_cli` and `check_serve` (see `src/doctor.rs`) both spawn
//! `std::env::current_exe()`, so they only mean anything when the process
//! running `doctor::run()` *is* the real `crucible` CLI binary — a unit test
//! inside `cargo test`'s own test-harness binary can't exercise them
//! meaningfully (`current_exe()` there resolves to the test harness, not the
//! CLI). This file is the doctor happy-path oracle crucible-911 asks for: it
//! spawns the actual built binary, runs `doctor --json`, and asserts every
//! check the acceptance criteria name.
//!
//! No test here makes a live OpenRouter call or requires network access: the
//! `model_credentials` check only ever inspects whether `OPENROUTER_API_KEY`
//! is *set*, never whether it is valid, so a fake, non-empty value stands in
//! for "present but untested" without spending a real request.

use std::path::PathBuf;
use std::process::Command;

fn crucible() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crucible"))
}

fn checks_by_id(
    report: &serde_json::Value,
) -> std::collections::HashMap<String, serde_json::Value> {
    report["checks"]
        .as_array()
        .expect("checks is an array")
        .iter()
        .map(|check| {
            (
                check["id"]
                    .as_str()
                    .expect("check id is a string")
                    .to_string(),
                check.clone(),
            )
        })
        .collect()
}

/// The headline test: `crucible doctor --json` with no `OPENROUTER_API_KEY`
/// set must report every functionality check (`cli`, `mcp`, `serve`,
/// `ledger`) as `ok`, `model_credentials` as `warn` (not `fail`), and the
/// top-level `ok` as `true` — a missing optional credential must never sink
/// the overall verdict.
#[test]
fn doctor_happy_path_without_openrouter_key_is_ok_overall_with_a_credentials_warning() {
    let out = crucible()
        .arg("doctor")
        .arg("--json")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "doctor must exit 0 when every functionality check passes, even with no \
         OPENROUTER_API_KEY; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("doctor --json must emit valid JSON");
    assert_eq!(report["schema_version"], "crucible.doctor_report.v1");
    assert_eq!(report["ok"], true, "report: {report}");

    let checks = checks_by_id(&report);
    for id in ["cli", "mcp", "serve", "ledger"] {
        let check = checks
            .get(id)
            .unwrap_or_else(|| panic!("missing {id} check: {report}"));
        assert_eq!(
            check["status"], "ok",
            "{id} check must be ok in a hermetic environment: {check}"
        );
    }
    let credentials = &checks["model_credentials"];
    assert_eq!(
        credentials["status"], "warn",
        "missing OPENROUTER_API_KEY must warn, not fail: {credentials}"
    );
    let message = credentials["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("OPENROUTER_API_KEY"),
        "message should name the env var: {message}"
    );
}

/// A present-but-fake credential must flip `model_credentials` to `ok`
/// without ever echoing the value anywhere in the report — this is the
/// "present but untested" stand-in for a real key: doctor only checks
/// presence, so no live OpenRouter call happens here.
#[test]
fn doctor_reports_ok_credentials_without_ever_echoing_the_value() {
    let fake_key = "doctor-test-value-should-never-appear-in-output";
    let out = crucible()
        .arg("doctor")
        .arg("--json")
        .env("OPENROUTER_API_KEY", fake_key)
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "doctor must exit 0 when every check passes; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(fake_key),
        "doctor must never print the credential value: {stdout}"
    );

    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("doctor --json must emit valid JSON");
    assert_eq!(report["ok"], true, "report: {report}");
    let checks = checks_by_id(&report);
    assert_eq!(checks["model_credentials"]["status"], "ok");
}

/// The human-readable (non-`--json`) report must also never leak a
/// credential value, and must still surface every check id.
#[test]
fn doctor_human_readable_report_lists_every_check_and_never_leaks_credentials() {
    let fake_key = "doctor-test-human-mode-value-should-not-leak";
    let out = crucible()
        .arg("doctor")
        .env("OPENROUTER_API_KEY", fake_key)
        .output()
        .expect("crucible binary runs");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(fake_key),
        "human report leaked a credential: {stdout}"
    );
    for id in ["cli", "mcp", "serve", "ledger", "model_credentials"] {
        assert!(
            stdout.contains(id),
            "human-readable report should mention check {id}: {stdout}"
        );
    }
}

/// The ledger check must actually create a SQLite file under `runs/` in the
/// crate's own working directory (the same relative-path convention the rest
/// of the CLI uses), not merely claim success in the JSON body — and it must
/// clean up after itself. This inspects the specific scratch path *this*
/// invocation's `ledger` check names in its own message (parsed out of the
/// message text) rather than diffing the shared `runs/doctor-check/`
/// directory, which other tests in this file touch concurrently.
#[test]
fn doctor_ledger_check_creates_then_cleans_up_its_own_scratch_file() {
    let out = crucible()
        .arg("doctor")
        .arg("--json")
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("crucible binary runs");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("doctor --json must emit valid JSON");
    let ledger = &checks_by_id(&report)["ledger"];
    assert_eq!(ledger["status"], "ok", "{ledger}");
    let message = ledger["message"]
        .as_str()
        .expect("ledger message is a string");
    let db_path = message
        .strip_prefix("created and opened ")
        .and_then(|rest| rest.split(" (").next())
        .unwrap_or_else(|| panic!("ledger message did not name a path: {message}"));

    // The path is relative to the crucible package directory (the same
    // convention every other relative-path default in this CLI uses), which
    // is also this integration test binary's own working directory.
    let full_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(db_path);
    assert!(
        !full_path.exists(),
        "doctor's ledger self-check must remove its scratch file after proving it can be \
         created: {}",
        full_path.display()
    );
}
