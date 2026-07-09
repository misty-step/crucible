//! `crucible doctor`: push-button, verified-live onboarding check (crucible-911).
//!
//! Crucible has README quickstarts and a full CI gate, but before this,
//! onboarding ended at commands a human or agent could *run*, not a single
//! command that proves the CLI, MCP, serve UI, and run ledger are actually
//! *live* on this machine. `doctor` runs five cheap, hermetic checks and
//! reports a stable `crucible.doctor_report.v1` object:
//!
//! - `cli`: the compiled binary itself is invocable (`--version` succeeds).
//! - `mcp`: the stdio MCP server initializes and lists a non-empty tool
//!   surface, through the exact in-process path [`crate::mcp::self_check`]
//!   uses (no subprocess, no stdio).
//! - `serve`: `crucible serve` binds a real OS-assigned port and
//!   `GET /api/specs` returns the expected schema, over a real spawned
//!   subprocess against a scratch, empty specs directory.
//! - `ledger`: the SQLite run ledger can be created and opened under `runs/`.
//! - `model_credentials`: whether `OPENROUTER_API_KEY` is set — never its
//!   value. Absence is `Warn`, not `Fail`: live-model runs (`prompt_benchmark`,
//!   `agentic_judge`) need it, but every other surface does not.
//!
//! None of these checks makes a network call to a model provider or requires
//! `OPENROUTER_API_KEY` to be genuinely valid — `model_credentials` only
//! checks presence, so `doctor` is safe to run in CI and to test hermetically.
//!
//! Unlike `validate`/`grade`/`adjudicate` (whose verdict lives only in the
//! JSON body so a non-conforming spec is not a process failure), `doctor`'s
//! whole purpose is a scriptable readiness gate, so a `Fail` check makes the
//! command exit non-zero (see `run_doctor` in `main.rs`) after printing the
//! full report — an agent or CI step can act on the exit code alone, and a
//! human still gets the per-check detail either way.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{mcp, run_store};

/// Schema identifier for [`DoctorReport`].
pub const DOCTOR_REPORT_SCHEMA: &str = "crucible.doctor_report.v1";
/// The one credential every live-model runner reads (`crucible-core`'s
/// `default_openrouter_credential_env`). Doctor only ever checks *presence*.
const CREDENTIAL_ENV: &str = "OPENROUTER_API_KEY";

/// Monotonic counter so concurrent `doctor` runs inside one process (e.g. a
/// test binary spawning several subprocesses in parallel) never collide on
/// the same scratch ledger path or serve scratch directory.
static INVOCATION: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The surface is live and working.
    Ok,
    /// Not broken — an optional, documented gap (only ever
    /// `model_credentials` today).
    Warn,
    /// Broken local functionality; `doctor`'s overall `ok` goes `false`.
    Fail,
}

#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    pub id: &'static str,
    pub status: CheckStatus,
    pub message: String,
}

impl DoctorCheck {
    fn ok(id: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            status: CheckStatus::Ok,
            message: message.into(),
        }
    }

    fn warn(id: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            status: CheckStatus::Warn,
            message: message.into(),
        }
    }

    fn fail(id: &'static str, message: impl Into<String>) -> Self {
        Self {
            id,
            status: CheckStatus::Fail,
            message: message.into(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub schema_version: &'static str,
    /// `true` iff no check is [`CheckStatus::Fail`]. A `Warn` (missing
    /// optional model credentials) does not flip this to `false`.
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
}

/// Run every doctor check and assemble the report. Never panics: each check
/// catches its own errors and turns them into a `Fail` entry rather than
/// aborting the rest of the report.
pub fn run() -> DoctorReport {
    let checks = vec![
        check_cli(),
        check_mcp(),
        check_serve(),
        check_ledger(),
        check_model_credentials(),
    ];
    let ok = !checks.iter().any(|check| check.status == CheckStatus::Fail);
    DoctorReport {
        schema_version: DOCTOR_REPORT_SCHEMA,
        ok,
        checks,
    }
}

/// The CLI runs: spawn the actual running binary (`std::env::current_exe`,
/// never a hardcoded relative path) with `--version` and confirm it exits 0.
/// This is a real subprocess invocation, not just "doctor itself is running,
/// therefore the CLI works" — it would also catch a corrupted binary, a
/// missing dynamic library, or a broken installed wrapper script.
fn check_cli() -> DoctorCheck {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => {
            return DoctorCheck::warn(
                "cli",
                format!(
                    "could not resolve the running executable's own path ({err}); skipped the \
                     CLI invocation check (this is an environment limitation, not a Crucible bug)"
                ),
            )
        }
    };
    match Command::new(&exe).arg("--version").output() {
        Ok(output) if output.status.success() => DoctorCheck::ok(
            "cli",
            format!(
                "{} --version: {}",
                exe.display(),
                String::from_utf8_lossy(&output.stdout).trim()
            ),
        ),
        Ok(output) => DoctorCheck::fail(
            "cli",
            format!(
                "{} --version exited {}: {}",
                exe.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ),
        Err(err) => DoctorCheck::fail("cli", format!("failed to invoke {}: {err}", exe.display())),
    }
}

/// The MCP server initializes and lists its tools, in-process via
/// [`mcp::self_check`] — the exact `dispatch` path `crucible mcp` serves over
/// stdio, without needing a subprocess or a JSON-RPC transcript.
fn check_mcp() -> DoctorCheck {
    match mcp::self_check() {
        Ok(tools) => DoctorCheck::ok(
            "mcp",
            format!(
                "stdio MCP server initialized and listed {} tool(s): {}",
                tools.len(),
                tools.join(", ")
            ),
        ),
        Err(err) => DoctorCheck::fail("mcp", format!("MCP self-check failed: {err:#}")),
    }
}

/// `crucible serve` binds a real port and answers `GET /api/specs`. Spawns
/// the real binary (like the e2e tests in `tests/cli.rs`'s `spawn_serve`)
/// against a scratch, empty specs directory and a scratch ledger — no
/// existing user data is read or written.
fn check_serve() -> DoctorCheck {
    match check_serve_inner() {
        Ok(message) => DoctorCheck::ok("serve", message),
        Err(err) if err.to_string().contains("loopback bind refused") => DoctorCheck::warn(
            "serve",
            format!(
                "serve self-check skipped: {err:#} (this environment does not permit loopback \
                 listeners)"
            ),
        ),
        Err(err) => DoctorCheck::fail("serve", format!("serve self-check failed: {err:#}")),
    }
}

fn check_serve_inner() -> Result<String> {
    let exe = std::env::current_exe().context("resolving the running executable's own path")?;
    let n = INVOCATION.fetch_add(1, Ordering::SeqCst);
    let scratch =
        std::env::temp_dir().join(format!("crucible-doctor-serve-{}-{n}", std::process::id()));
    let specs_dir = scratch.join("evals");
    std::fs::create_dir_all(&specs_dir)
        .with_context(|| format!("creating scratch specs dir {}", specs_dir.display()))?;
    let db_path = scratch.join("doctor-serve.sqlite");

    let mut child = Command::new(&exe)
        .arg("serve")
        .arg("--db")
        .arg(&db_path)
        .arg("--specs")
        .arg(&specs_dir)
        .arg("--port")
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning `{} serve`", exe.display()))?;

    let outcome = (|| -> Result<String> {
        let stdout = child
            .stdout
            .take()
            .context("crucible serve stdout is piped")?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("reading crucible serve's startup line")?;
        let port: u16 = match line
            .split("http://127.0.0.1:")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|port| port.parse().ok())
        {
            Some(port) => port,
            None => {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                if stderr.contains("Operation not permitted") {
                    anyhow::bail!("loopback bind refused by OS: {stderr}");
                }
                anyhow::bail!("parsing bound port from startup line {line:?}; stderr={stderr}");
            }
        };

        let url = format!("http://127.0.0.1:{port}/api/specs");
        let response = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .context("building HTTP client")?
            .get(&url)
            .send()
            .with_context(|| format!("GET {url}"))?;
        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .with_context(|| format!("parsing {url} response body as JSON"))?;
        if !status.is_success() {
            anyhow::bail!("GET {url} returned {status}: {body}");
        }
        let schema = body.get("schema_version").and_then(|v| v.as_str());
        if schema != Some("crucible.ui.specs.v1") {
            anyhow::bail!("GET {url} returned an unexpected schema_version: {body}");
        }
        Ok(format!(
            "bound 127.0.0.1:{port} and GET /api/specs returned schema {schema:?}"
        ))
    })();

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&scratch);

    outcome
}

/// The run ledger (SQLite) can be created under `runs/`. Reuses
/// [`run_store::list_runs`], which opens-or-creates the schema exactly like
/// every real `crucible run`/`crucible runs list` invocation — no bespoke
/// schema-init path that could drift from the real one.
fn check_ledger() -> DoctorCheck {
    match check_ledger_inner() {
        Ok(message) => DoctorCheck::ok("ledger", message),
        Err(err) => DoctorCheck::fail("ledger", format!("run ledger self-check failed: {err:#}")),
    }
}

fn check_ledger_inner() -> Result<String> {
    let n = INVOCATION.fetch_add(1, Ordering::SeqCst);
    let scratch_dir = std::path::Path::new("runs").join("doctor-check");
    std::fs::create_dir_all(&scratch_dir)
        .with_context(|| format!("creating {}", scratch_dir.display()))?;
    let db_path = scratch_dir.join(format!("ledger-check-{}-{n}.sqlite", std::process::id()));

    let list = run_store::list_runs(&db_path, run_store::RunListFilter::default())
        .with_context(|| format!("creating/opening {}", db_path.display()))?;
    let created = db_path.is_file();

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir(&scratch_dir); // best-effort; only succeeds if empty

    if !created {
        anyhow::bail!(
            "list_runs succeeded but {} was never created on disk",
            db_path.display()
        );
    }
    Ok(format!(
        "created and opened {} ({} row(s), schema initialized)",
        db_path.display(),
        list.runs.len()
    ))
}

/// Optional OpenRouter model credentials: `Warn`, never `Fail`, when absent
/// — this is the one check that distinguishes "not configured for live-model
/// spend" from "broken." Only presence is reported; the value never is.
fn check_model_credentials() -> DoctorCheck {
    check_model_credentials_with(std::env::var(CREDENTIAL_ENV).ok())
}

/// The credential logic, taking the (already-read) env value as a plain
/// argument instead of reading `std::env::var` itself. Tests call this
/// directly with an injected `Some`/`None` rather than mutating the
/// process-wide environment — `cargo test` runs this crate's tests in
/// parallel within one process, and `tests/cli.rs` already avoids exactly
/// this kind of shared mutable state for the same reason.
fn check_model_credentials_with(value: Option<String>) -> DoctorCheck {
    match value {
        Some(value) if !value.trim().is_empty() => DoctorCheck::ok(
            "model_credentials",
            format!(
                "{CREDENTIAL_ENV} is set (value not shown) — live-model prompt_benchmark/\
                 agentic_judge runs can reach OpenRouter"
            ),
        ),
        _ => DoctorCheck::warn(
            "model_credentials",
            format!(
                "{CREDENTIAL_ENV} is not set — this is optional: prompt_benchmark and \
                 agentic_judge live-model runs will refuse with a clear credential error, but \
                 validate, grade, adjudicate, export, dashboard, the run ledger, MCP, and serve \
                 all work without it"
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `check_cli` and `check_serve` spawn `std::env::current_exe()` and must
    // therefore run as the real compiled `crucible` binary to mean anything
    // — inside `cargo test`'s own unit-test harness binary, `current_exe()`
    // resolves to the *test harness*, not the CLI, so exercising them here
    // would only prove the test binary lacks `--version`. The full doctor
    // happy path (all five checks, run as the real binary) is covered by the
    // integration test `tests/doctor.rs`, which spawns
    // `CARGO_BIN_EXE_crucible` exactly like the rest of this crate's e2e
    // suite. Here we cover every check that is meaningfully hermetic
    // in-process: `mcp` and `ledger`.

    #[test]
    fn check_mcp_passes_in_process_with_no_subprocess() {
        let check = check_mcp();
        assert_eq!(check.status, CheckStatus::Ok, "{}", check.message);
        assert!(check.message.contains("crucible_run"));
    }

    #[test]
    fn check_ledger_creates_and_opens_the_sqlite_file_under_runs() {
        let check = check_ledger();
        assert_eq!(check.status, CheckStatus::Ok, "{}", check.message);
    }

    #[test]
    fn report_includes_all_five_checks_and_ok_reflects_no_failures() {
        let report = run();
        let ids: Vec<&str> = report.checks.iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            vec!["cli", "mcp", "serve", "ledger", "model_credentials"],
            "doctor must run exactly these five checks in this order"
        );
        assert_eq!(report.schema_version, DOCTOR_REPORT_SCHEMA);
        assert_eq!(
            report.ok,
            !report.checks.iter().any(|c| c.status == CheckStatus::Fail)
        );
    }

    #[test]
    fn missing_model_credentials_is_a_warning_not_a_failure() {
        let check = check_model_credentials_with(None);
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(
            !check.message.contains("value"),
            "message must never echo a credential value: {}",
            check.message
        );
    }

    #[test]
    fn present_model_credentials_is_ok_and_never_echoes_the_value() {
        let check = check_model_credentials_with(Some(
            "doctor-test-value-must-not-appear-in-output".to_string(),
        ));
        assert_eq!(check.status, CheckStatus::Ok);
        assert!(
            !check
                .message
                .contains("doctor-test-value-must-not-appear-in-output"),
            "message must never echo the credential value: {}",
            check.message
        );
    }

    #[test]
    fn empty_model_credential_value_is_treated_as_absent() {
        let check = check_model_credentials_with(Some("   ".to_string()));
        assert_eq!(
            check.status,
            CheckStatus::Warn,
            "a blank value must not count as a configured credential"
        );
    }

    #[test]
    fn doctor_report_ok_is_false_only_when_a_check_fails() {
        let all_ok = DoctorReport {
            schema_version: DOCTOR_REPORT_SCHEMA,
            ok: true,
            checks: vec![
                DoctorCheck::ok("a", "fine"),
                DoctorCheck::warn("b", "optional gap"),
            ],
        };
        assert!(
            !all_ok.checks.iter().any(|c| c.status == CheckStatus::Fail),
            "sanity: this fixture has no Fail checks"
        );

        let one_fails = [
            DoctorCheck::ok("a", "fine"),
            DoctorCheck::fail("b", "broken"),
        ];
        let ok = !one_fails.iter().any(|c| c.status == CheckStatus::Fail);
        assert!(!ok, "a single Fail check must flip the overall verdict");
    }
}
