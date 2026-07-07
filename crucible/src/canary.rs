//! Fire-and-forget Canary self-reporter. No creds => silent no-op.
//!
//! A Canary outage never blocks, slows, or panics crucible. Sends run on a
//! detached thread with a bounded per-attempt timeout and a single retry;
//! every failure is swallowed.
//!
//! `crucible` is mostly a one-shot CLI (`check_in()` fires once per
//! invocation; an overdue monitor between runs is expected, not an
//! incident), but `serve`/`mcp` are standing services that outlive the
//! check-in TTL — those bootstraps call [`start_health_loop`] instead so the
//! monitor never goes falsely overdue while the process is healthy and
//! running.
//!
//! Comprehensive coverage (`docs/rust-consumer-integration.md`) also wires:
//! - [`CanaryLayer`], a `tracing_subscriber::Layer` that forwards every
//!   `ERROR`-level event to [`report_error`] with zero per-site call sites.
//! - [`install_panic_hook`] / [`report_panic`], which turn an unhandled or
//!   caught panic into a `<service>.panic` error report.
//!
//! Because sends happen on a detached thread, a short-lived CLI process can
//! exit before the send lands. `flush()` blocks the caller for a bounded
//! window so an in-flight check-in or error report reaches the network
//! before `main` returns; it must be called right before process exit.
//!
//! Uses `reqwest::blocking` (already a workspace dependency with the
//! `blocking` feature enabled) rather than adding `ureq`, per
//! `docs/rust-consumer-integration.md`'s zero-new-dependency fallback.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

const SERVICE: &str = "crucible"; // overridable via CANARY_SERVICE
const MONITOR: &str = "crucible"; // must already exist in Canary (MON-sbcmhg2rt2s6)
const TTL_MS: u64 = 120_000;
const SEND_TIMEOUT: Duration = Duration::from_secs(3);
// Two attempts at SEND_TIMEOUT each, plus a small margin for client
// construction — the bound `flush()` waits before giving up on an in-flight
// send.
const FLUSH_BUDGET: Duration = Duration::from_secs(7);
// Standing-service health loop: fire once immediately (in
// `start_health_loop`), then re-check-in on this cadence from a named
// background thread. `TTL_MS` is 2x this interval, matching the reference
// pattern in `docs/rust-consumer-integration.md`.
const CHECKIN_INTERVAL: Duration = Duration::from_secs(60);

fn config() -> Option<(String, String)> {
    let endpoint = std::env::var("CANARY_ENDPOINT").ok()?;
    let key = std::env::var("CANARY_API_KEY")
        .or_else(|_| std::env::var("CANARY_INGEST_KEY"))
        .ok()?;
    (!endpoint.trim().is_empty() && !key.trim().is_empty())
        .then(|| (endpoint.trim_end_matches('/').to_owned(), key))
}

fn service() -> String {
    std::env::var("CANARY_SERVICE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| SERVICE.to_owned())
}

/// In-flight send counter + condvar, so `flush()` can wait for outstanding
/// sends without holding a `JoinHandle` per call site.
fn inflight() -> &'static (Mutex<u32>, Condvar) {
    static INFLIGHT: OnceLock<(Mutex<u32>, Condvar)> = OnceLock::new();
    INFLIGHT.get_or_init(|| (Mutex::new(0), Condvar::new()))
}

/// Report a handled or unhandled error. Safe to call anywhere; no-ops
/// silently when Canary creds are absent.
pub fn report_error(error_class: &str, message: &str) {
    let Some((endpoint, key)) = config() else {
        return;
    };
    let environment =
        std::env::var("CANARY_ENVIRONMENT").unwrap_or_else(|_| "production".to_owned());
    let body = serde_json::json!({
        "service": service(),
        "error_class": error_class,
        "message": message.chars().take(4096).collect::<String>(),
        "severity": "error",
        "environment": environment,
    });
    spawn_send(endpoint, key, "/api/v1/errors", body);
}

/// Heartbeat: one check-in per invocation. No background loop — crucible is
/// a CLI/build tool, not a standing service.
pub fn check_in() {
    let Some((endpoint, key)) = config() else {
        return;
    };
    let body = serde_json::json!({
        "monitor": MONITOR,
        "status": "alive",
        "summary": concat!(env!("CARGO_PKG_NAME"), " run"),
        "ttl_ms": TTL_MS,
    });
    spawn_send(endpoint, key, "/api/v1/check-ins", body);
}

/// Standing services only: fire a check-in immediately, then re-check-in
/// every [`CHECKIN_INTERVAL`] from a named background thread for the life of
/// the process. Call this at the top of *every* long-running bootstrap
/// (`serve`, `mcp`, any future daemon entry) — a one-shot [`check_in`] is not
/// enough for a process that outlives [`TTL_MS`]; it goes falsely overdue.
/// No-ops (spawns nothing) without Canary credentials.
pub fn start_health_loop() {
    if config().is_none() {
        return;
    }
    check_in();
    let _ = std::thread::Builder::new()
        .name("canary-health".into())
        .spawn(|| loop {
            std::thread::sleep(CHECKIN_INTERVAL);
            check_in();
        });
}

/// Forwards every `ERROR`-level tracing event to [`report_error`], so
/// `tracing::error!(...)` anywhere in the app (or a library it calls) is
/// captured with zero per-site wiring. Register alongside the fmt layer:
/// `tracing_subscriber::registry().with(fmt_layer).with(canary::CanaryLayer).init()`.
/// No-ops when Canary creds are absent.
pub struct CanaryLayer;

impl<S: Subscriber> Layer<S> for CanaryLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if *event.metadata().level() != Level::ERROR || config().is_none() {
            return;
        }
        let mut message = String::new();
        event.record(&mut FieldVisitor(&mut message));
        let class = format!("{}.{}", service(), event.metadata().target());
        report_error(&class, &message);
    }
}

struct FieldVisitor<'a>(&'a mut String);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        self.0.push_str(&format!("{}={value:?}", field.name()));
    }
}

/// Install a global panic hook that reports `<service>.panic` to Canary,
/// flushes it, then chains to the previous hook so default panic output
/// (backtrace, process exit behavior) is unchanged. Call once at process
/// start. No-ops — never touches `std::panic::set_hook` — without Canary
/// credentials.
pub fn install_panic_hook() {
    if config().is_none() {
        return;
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let message = format!("{} @ {loc}", panic_message(info.payload()));
        report_error(&panic_class(), &message);
        flush(); // best-effort before the process dies
        default_hook(info);
    }));
}

/// Report an already-caught panic payload (e.g. from a `catch_unwind`
/// boundary around a request/connection handler) as `<service>.panic`. Safe
/// to call anywhere; no-ops silently when Canary creds are absent, via
/// [`report_error`].
pub fn report_panic(payload: &(dyn std::any::Any + Send)) {
    report_error(&panic_class(), &panic_message(payload));
}

fn panic_class() -> String {
    format!("{}.panic", service())
}

/// Extract a human-readable message from a panic payload. Covers the two
/// payload shapes `panic!`/`unwrap`/`expect` actually produce (`&str` and
/// `String`); anything else falls back to a generic marker rather than
/// failing to report at all.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_owned())
}

/// Block briefly for any in-flight send to land before the process exits.
/// Bounded by `FLUSH_BUDGET` so a dead or slow Canary endpoint never hangs a
/// run. No-op when nothing is in flight (including when creds are absent,
/// since `spawn_send` is never reached).
pub fn flush() {
    let (lock, cvar) = inflight();
    let Ok(guard) = lock.lock() else {
        return;
    };
    let _ = cvar.wait_timeout_while(guard, FLUSH_BUDGET, |count| *count > 0);
}

fn release_inflight_slot() {
    let (lock, cvar) = inflight();
    if let Ok(mut count) = lock.lock() {
        *count = count.saturating_sub(1);
    }
    cvar.notify_all();
}

fn spawn_send(endpoint: String, key: String, path: &'static str, body: serde_json::Value) {
    let (lock, _cvar) = inflight();
    if let Ok(mut count) = lock.lock() {
        *count += 1;
    } else {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("canary-report".into())
        .spawn(move || {
            send_with_retry(&endpoint, &key, path, &body);
            release_inflight_slot();
        });
    if spawned.is_err() {
        // Thread spawn itself failed (resource exhaustion) — release the
        // slot reserved above so flush() never hangs waiting on it.
        release_inflight_slot();
    }
}

fn send_with_retry(endpoint: &str, key: &str, path: &str, body: &serde_json::Value) {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(SEND_TIMEOUT)
        .build()
    else {
        return;
    };
    let url = format!("{endpoint}{path}");
    for _ in 0..2 {
        // one retry, then give up silently
        let sent = client
            .post(&url)
            .bearer_auth(key)
            .json(body)
            .send()
            .is_ok_and(|resp| resp.status().is_success());
        if sent {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::time::Instant;

    /// `check_in`/`report_error` read process-global `CANARY_*` env vars, and
    /// `cargo test` runs this crate's tests in parallel within one process
    /// (see `doctor::check_model_credentials_with`'s comment for the same
    /// concern) — serialize every test below on this lock so their env
    /// mutations never interleave.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct CapturedRequest {
        request_line: String,
        authorization: Option<String>,
        body: serde_json::Value,
    }

    fn handle_connection(stream: TcpStream) -> CapturedRequest {
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .expect("read request line");

        let mut content_length = 0usize;
        let mut authorization = None;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read header line");
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.trim_end().split_once(": ") {
                match name.to_ascii_lowercase().as_str() {
                    "content-length" => content_length = value.parse().unwrap_or(0),
                    "authorization" => authorization = Some(value.to_string()),
                    _ => {}
                }
            }
        }

        let mut body_bytes = vec![0u8; content_length];
        reader.read_exact(&mut body_bytes).expect("read body");
        let body: serde_json::Value =
            serde_json::from_slice(&body_bytes).expect("body is valid JSON");

        let mut stream = reader.into_inner();
        let _ = stream
            .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");

        CapturedRequest {
            request_line: request_line.trim().to_string(),
            authorization,
            body,
        }
    }

    /// Bind an ephemeral mock server that captures exactly one request per
    /// accepted connection and replies `201 Created`.
    fn spawn_mock_server() -> (String, mpsc::Receiver<CapturedRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
        let addr = listener.local_addr().expect("mock listener local addr");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(stream) = incoming else { continue };
                let captured = handle_connection(stream);
                if tx.send(captured).is_err() {
                    break;
                }
            }
        });
        (format!("http://{addr}"), rx)
    }

    fn clear_canary_env() {
        // SAFETY: every caller holds `ENV_LOCK` for the duration of its env
        // mutations, so no other thread observes a torn env in between.
        unsafe {
            std::env::remove_var("CANARY_ENDPOINT");
            std::env::remove_var("CANARY_API_KEY");
            std::env::remove_var("CANARY_INGEST_KEY");
            std::env::remove_var("CANARY_SERVICE");
            std::env::remove_var("CANARY_ENVIRONMENT");
        }
    }

    #[test]
    fn check_in_posts_monitor_status_and_ttl_to_the_mock_server() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_canary_env();
        let (endpoint, rx) = spawn_mock_server();
        // SAFETY: serialized by `ENV_LOCK` above.
        unsafe {
            std::env::set_var("CANARY_ENDPOINT", &endpoint);
            std::env::set_var("CANARY_API_KEY", "test-key");
        }

        check_in();
        flush();

        let captured = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("mock server received a check-in request");
        clear_canary_env();

        assert_eq!(captured.request_line, "POST /api/v1/check-ins HTTP/1.1");
        assert_eq!(captured.authorization.as_deref(), Some("Bearer test-key"));
        assert_eq!(captured.body["monitor"], "crucible");
        assert_eq!(captured.body["status"], "alive");
        assert_eq!(captured.body["ttl_ms"], 120_000);
    }

    #[test]
    fn report_error_posts_service_class_and_message_to_the_mock_server() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_canary_env();
        let (endpoint, rx) = spawn_mock_server();
        // SAFETY: serialized by `ENV_LOCK` above.
        unsafe {
            std::env::set_var("CANARY_ENDPOINT", &endpoint);
            std::env::set_var("CANARY_API_KEY", "test-key");
        }

        report_error("crucible.run.failed", "boom");
        flush();

        let captured = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("mock server received an error report");
        clear_canary_env();

        assert_eq!(captured.request_line, "POST /api/v1/errors HTTP/1.1");
        assert_eq!(captured.authorization.as_deref(), Some("Bearer test-key"));
        assert_eq!(captured.body["service"], "crucible");
        assert_eq!(captured.body["error_class"], "crucible.run.failed");
        assert_eq!(captured.body["message"], "boom");
        assert_eq!(captured.body["severity"], "error");
    }

    #[test]
    fn without_credentials_check_in_and_report_error_are_silent_no_ops() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_canary_env();

        // No panic, no send, and flush() returns immediately since nothing
        // was ever put in flight.
        check_in();
        report_error("crucible.test.no_creds", "should never send");
        flush();
    }

    #[test]
    fn report_error_against_a_dead_port_returns_promptly_without_panicking() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_canary_env();
        // Bind then drop to reserve a port that is guaranteed closed for the
        // rest of this test.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind throwaway listener");
        let addr = listener
            .local_addr()
            .expect("throwaway listener local addr");
        drop(listener);
        // SAFETY: serialized by `ENV_LOCK` above.
        unsafe {
            std::env::set_var("CANARY_ENDPOINT", format!("http://{addr}"));
            std::env::set_var("CANARY_API_KEY", "test-key");
        }

        let started = Instant::now();
        report_error("crucible.test.dead_port", "probe");
        flush();
        let elapsed = started.elapsed();
        clear_canary_env();

        assert!(
            elapsed < Duration::from_secs(15),
            "flush() must return within a bounded window against a dead port, took {elapsed:?}"
        );
    }

    #[test]
    fn canary_layer_forwards_a_tracing_error_event_to_the_mock_server() {
        use tracing_subscriber::prelude::*;

        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_canary_env();
        let (endpoint, rx) = spawn_mock_server();
        // SAFETY: serialized by `ENV_LOCK` above.
        unsafe {
            std::env::set_var("CANARY_ENDPOINT", &endpoint);
            std::env::set_var("CANARY_API_KEY", "test-key");
        }

        // Thread-local subscriber override: scoped to this closure, so it
        // never touches global state other parallel tests rely on.
        let subscriber = tracing_subscriber::registry().with(CanaryLayer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!("boom from the layer");
        });
        flush();

        let captured = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("mock server received an error forwarded by CanaryLayer");
        clear_canary_env();

        assert_eq!(captured.request_line, "POST /api/v1/errors HTTP/1.1");
        assert_eq!(captured.body["service"], "crucible");
        assert_eq!(captured.body["severity"], "error");
        assert!(
            captured.body["message"]
                .as_str()
                .expect("message is a string")
                .contains("boom from the layer"),
            "message must carry the event's text: {}",
            captured.body["message"]
        );
        assert!(
            captured.body["error_class"]
                .as_str()
                .expect("error_class is a string")
                .starts_with("crucible."),
            "error_class must be service-scoped: {}",
            captured.body["error_class"]
        );
    }

    #[test]
    fn canary_layer_is_a_silent_no_op_without_credentials() {
        use tracing_subscriber::prelude::*;

        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_canary_env();

        let subscriber = tracing_subscriber::registry().with(CanaryLayer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!("should never be sent");
        });
        // No panic, no send, and flush() returns immediately since nothing
        // was ever put in flight.
        flush();
    }

    #[test]
    fn install_panic_hook_is_a_no_op_without_credentials() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_canary_env();
        // Without creds this returns before touching std::panic::set_hook,
        // so it is safe to call from a parallel test run.
        install_panic_hook();
    }

    #[test]
    fn panic_message_extracts_str_and_string_payloads_and_falls_back_otherwise() {
        let str_payload: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_message(str_payload.as_ref()), "boom");

        let string_payload: Box<dyn std::any::Any + Send> = Box::new(String::from("kaboom"));
        assert_eq!(panic_message(string_payload.as_ref()), "kaboom");

        let other_payload: Box<dyn std::any::Any + Send> = Box::new(42_i32);
        assert_eq!(
            panic_message(other_payload.as_ref()),
            "panic",
            "unrecognized payload shapes fall back to a generic marker, not a failure to report"
        );
    }

    #[test]
    fn panic_class_is_service_scoped() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_canary_env();
        assert_eq!(panic_class(), "crucible.panic");
    }

    #[test]
    fn report_panic_posts_a_service_scoped_panic_class_to_the_mock_server() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_canary_env();
        let (endpoint, rx) = spawn_mock_server();
        // SAFETY: serialized by `ENV_LOCK` above.
        unsafe {
            std::env::set_var("CANARY_ENDPOINT", &endpoint);
            std::env::set_var("CANARY_API_KEY", "test-key");
        }

        let payload: Box<dyn std::any::Any + Send> = Box::new("connection handler panicked");
        report_panic(payload.as_ref());
        flush();

        let captured = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("mock server received the panic report");
        clear_canary_env();

        assert_eq!(captured.body["error_class"], "crucible.panic");
        assert_eq!(captured.body["message"], "connection handler panicked");
    }
}
