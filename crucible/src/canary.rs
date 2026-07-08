//! Fire-and-forget Canary self-reporter. No creds => silent no-op.
//!
//! A Canary outage never blocks, slows, or panics crucible. Sends run on a
//! detached thread with a bounded per-attempt timeout and a single retry;
//! every failure is swallowed. Crucible is a CLI, not a standing service, so
//! there is no background health loop — `check_in()` fires once per
//! invocation and an overdue monitor between runs is expected, not an
//! incident.
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

const SERVICE: &str = "crucible"; // overridable via CANARY_SERVICE
const MONITOR: &str = "crucible"; // must already exist in Canary (MON-sbcmhg2rt2s6)
const TTL_MS: u64 = 120_000;
const SEND_TIMEOUT: Duration = Duration::from_secs(3);
// Two attempts at SEND_TIMEOUT each, plus a small margin for client
// construction — the bound `flush()` waits before giving up on an in-flight
// send.
const FLUSH_BUDGET: Duration = Duration::from_secs(7);

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
}
