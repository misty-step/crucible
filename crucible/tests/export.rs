//! Golden round-trip test for `crucible export` (epic 002.5).
//!
//! Drives the built binary as a subprocess (no `assert_cmd`; Cargo hands the
//! path in `CARGO_BIN_EXE_crucible`) over a committed labeled judgment queue,
//! and pins the emitted `adjudications.md` byte-for-byte against the golden
//! fixture. The round-trip is then closed in two directions through the public
//! `crucible_core` API: the golden parses back into the adjudication set, and
//! re-rendering that set reproduces the golden exactly — proving render and
//! parse are mutual inverses on the real artifact.
//!
//! The fixture mirrors the real pr-review-v0 py-file-cache scenario: one ACCEPT
//! (the tmp-write-race finding, a multi-line claim that exercises newline
//! escaping; the real ADJ-1) that bumps the arena `0.2.0 → 0.3.0` and extends the
//! key, plus two OUT-OF-SCOPE rulings — a correct-but-out-of-contract finding
//! (the real ADJ-2) and a confirmed-noise nit whose label carries an empty
//! timestamp.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use crucible_core::{parse_adjudications_md, render_adjudications_md, Ruling, Verdict};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn crucible() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crucible"))
}

/// A fresh, unique output directory under the system temp dir.
fn temp_out(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("crucible-export-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp out dir");
    dir
}

/// The headline golden test: the CLI's `adjudications.md` is byte-identical to
/// the committed golden, and that golden round-trips losslessly through
/// `parse` → `render`.
#[test]
fn export_matches_golden_and_round_trips_losslessly() {
    let out = temp_out("golden");
    let run = crucible()
        .arg("export")
        .arg("--labels")
        .arg(fixture("export-queue.json"))
        .arg("--out")
        .arg(&out)
        .arg("--arena")
        .arg("pr-review-v0")
        .arg("--task")
        .arg("py-file-cache")
        .arg("--base-version")
        .arg("0.2.0")
        .arg("--date")
        .arg("2026-06-29")
        .arg("--key")
        .arg(fixture("export-original-key.json"))
        .output()
        .expect("crucible binary runs");
    assert!(
        run.status.success(),
        "export must exit 0; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    // 1. The emitted log is byte-identical to the committed golden.
    let emitted = std::fs::read_to_string(out.join("adjudications.md")).expect("read emitted md");
    let golden =
        std::fs::read_to_string(fixture("export-adjudications.golden.md")).expect("golden");
    assert_eq!(
        emitted, golden,
        "the CLI's adjudications.md must match the committed golden byte-for-byte"
    );

    // 2. The golden parses back, and re-rendering is a fixpoint — render and
    //    parse are mutual inverses on the real artifact.
    let parsed = parse_adjudications_md(&golden).expect("golden adjudications.md parses");
    assert_eq!(parsed.arena, "pr-review-v0");
    assert_eq!(
        parsed.adjudications.len(),
        3,
        "three adjudications round-trip"
    );
    let rerendered = render_adjudications_md(&parsed.arena, &parsed.adjudications);
    assert_eq!(
        rerendered, golden,
        "render(parse(golden)) must reproduce the golden — the round-trip is lossless"
    );

    // 3. The ACCEPT carries the real version bump and a multi-line claim.
    let accept = &parsed.adjudications[0];
    assert_eq!(accept.finding_id, "F3");
    assert_eq!(accept.verdict, Verdict::Keep);
    assert!(accept.disposition.in_scope);
    match accept.ruling {
        Ruling::Accept { from, to } => {
            assert_eq!(from.to_string(), "0.2.0");
            assert_eq!(to.to_string(), "0.3.0");
        }
        Ruling::OutOfScope => panic!("ADJ-1 must be an ACCEPT"),
    }
    assert!(
        accept.description.contains('\n'),
        "the escaped newline round-trips back to a real newline in the claim"
    );

    // 4. The correct-but-out-of-contract finding and the confirmed nit are both
    //    OUT-OF-SCOPE; the empty timestamp survives.
    let out_of_contract = &parsed.adjudications[1];
    assert_eq!(out_of_contract.verdict, Verdict::Keep);
    assert!(!out_of_contract.disposition.in_scope);
    assert!(matches!(out_of_contract.ruling, Ruling::OutOfScope));

    let nit = &parsed.adjudications[2];
    assert_eq!(nit.verdict, Verdict::Noise);
    assert!(matches!(nit.ruling, Ruling::OutOfScope));
    assert_eq!(
        nit.conditions.timestamp, "",
        "an empty timestamp round-trips as empty"
    );

    let _ = std::fs::remove_dir_all(&out);
}

/// With `--key`, the Harbor `solution/findings.json` oracle is extended with the
/// accepted finding (and only it), and the appended row carries no `source_id`.
#[test]
fn export_extends_the_harbor_oracle_key_with_accepts_only() {
    let out = temp_out("key");
    let run = crucible()
        .arg("export")
        .arg("--labels")
        .arg(fixture("export-queue.json"))
        .arg("--out")
        .arg(&out)
        .arg("--arena")
        .arg("pr-review-v0")
        .arg("--task")
        .arg("py-file-cache")
        .arg("--base-version")
        .arg("0.2.0")
        .arg("--date")
        .arg("2026-06-29")
        .arg("--key")
        .arg(fixture("export-original-key.json"))
        .output()
        .expect("crucible binary runs");
    assert!(run.status.success(), "export must exit 0");

    let key_json =
        std::fs::read_to_string(out.join("solution/findings.json")).expect("read extended key");
    let key: serde_json::Value = serde_json::from_str(&key_json).expect("extended key is JSON");
    let findings = key["findings"].as_array().expect("findings array");
    assert_eq!(
        findings.len(),
        3,
        "two original rows plus the one accepted finding"
    );
    let appended = &findings[2];
    assert_eq!(appended["category"], "concurrency");
    assert_eq!(appended["line"], 23);
    assert!(
        appended.get("source_id").is_none(),
        "an answer-key row carries no source id: {appended}"
    );

    // The two severity-less source rows must re-emit byte-faithfully: a real
    // Daedalus key that omits `severity` must not grow a spurious
    // `"severity": ""` on round-trip (the real-key serde-stability invariant),
    // so the extension only ever appends the accepted row and never mutates the
    // originals.
    let original: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixture("export-original-key.json")).expect("read original key"),
    )
    .expect("original key is JSON");
    let original_rows = original["findings"].as_array().expect("original findings");
    assert_eq!(
        &findings[..2],
        &original_rows[..],
        "the severity-less source rows must re-emit unchanged"
    );
    assert!(
        findings[..2].iter().all(|r| r.get("severity").is_none()),
        "a severity-less source row must not gain a \"severity\" key: {key}"
    );

    let _ = std::fs::remove_dir_all(&out);
}

/// With `--expected`, the `tests/expected.json` **scorer key** (the file
/// `daedalus-score` reads) is extended with the accepted finding as a line-span
/// defect: the original seeded defects are preserved verbatim, and the appended
/// defect carries a one-line span (`line_start == line_end == line`), a
/// deterministic slug id, `note` = the description, and **no** `severity` key —
/// the exact shape that makes an accepted finding re-score as a true positive.
#[test]
fn export_extends_the_scorer_key_with_accepted_defects() {
    let out = temp_out("expected");
    let run = crucible()
        .arg("export")
        .arg("--labels")
        .arg(fixture("export-queue.json"))
        .arg("--out")
        .arg(&out)
        .arg("--arena")
        .arg("pr-review-v0")
        .arg("--task")
        .arg("py-file-cache")
        .arg("--base-version")
        .arg("0.2.0")
        .arg("--date")
        .arg("2026-06-29")
        .arg("--expected")
        .arg(fixture("export-original-expected.json"))
        .output()
        .expect("crucible binary runs");
    assert!(
        run.status.success(),
        "export must exit 0; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let expected_json =
        std::fs::read_to_string(out.join("tests/expected.json")).expect("read extended scorer key");
    let key: serde_json::Value =
        serde_json::from_str(&expected_json).expect("extended scorer key is JSON");
    let defects = key["defects"].as_array().expect("defects array");
    assert_eq!(
        defects.len(),
        3,
        "two seeded defects plus the one accepted finding: {key}"
    );

    // The two seeded defects are preserved verbatim, in order.
    assert_eq!(defects[0]["id"], "path-traversal");
    assert_eq!(defects[1]["id"], "unclosed-file");

    // The accepted finding lands as a one-line span with a slug id and a note,
    // and carries NO severity (so the scorer's severity gate never drops it).
    let added = &defects[2];
    assert_eq!(
        added["id"], "concurrency-cache-py-23",
        "deterministic slug id"
    );
    assert_eq!(added["file"], "cache.py");
    assert_eq!(
        added["line_start"], 23,
        "the point anchor collapses to a one-line span"
    );
    assert_eq!(added["line_end"], 23);
    assert_eq!(added["category"], "concurrency");
    assert!(
        added["note"].as_str().is_some_and(|n| !n.is_empty()),
        "note carries the finding description: {added}"
    );
    assert!(
        added.get("severity").is_none(),
        "the accepted defect omits severity so the finding matches on file+category+span: {added}"
    );

    let _ = std::fs::remove_dir_all(&out);
}

/// Without `--key`, only `adjudications.md` is written — an out-of-scope-leaning
/// run leaves no Harbor key behind.
#[test]
fn export_without_key_writes_only_the_log() {
    let out = temp_out("nokey");
    let run = crucible()
        .arg("export")
        .arg("--labels")
        .arg(fixture("export-queue.json"))
        .arg("--out")
        .arg(&out)
        .arg("--arena")
        .arg("pr-review-v0")
        .arg("--task")
        .arg("py-file-cache")
        .arg("--base-version")
        .arg("0.2.0")
        .arg("--date")
        .arg("2026-06-29")
        .output()
        .expect("crucible binary runs");
    assert!(run.status.success(), "export must exit 0");
    assert!(
        out.join("adjudications.md").exists(),
        "the log is always written"
    );
    assert!(
        !out.join("solution").exists(),
        "no --key means no Harbor oracle is written"
    );
    assert!(
        !out.join("tests").exists(),
        "no --expected means no scorer key is written"
    );

    let _ = std::fs::remove_dir_all(&out);
}

/// A bad `--key` fails fast and leaves NO half-written tree. Every output is
/// rendered before anything is written, so a key that cannot load aborts the run
/// before `adjudications.md` — which would otherwise assert an ACCEPT and a
/// version bump that never landed — is committed.
#[test]
fn export_with_bad_key_writes_nothing() {
    let out = temp_out("badkey");
    let run = crucible()
        .arg("export")
        .arg("--labels")
        .arg(fixture("export-queue.json"))
        .arg("--out")
        .arg(&out)
        .arg("--arena")
        .arg("pr-review-v0")
        .arg("--task")
        .arg("py-file-cache")
        .arg("--base-version")
        .arg("0.2.0")
        // A --key that cannot load: the log must not be written despite this.
        .arg("--key")
        .arg(out.join("does-not-exist.json"))
        .output()
        .expect("crucible binary runs");
    assert_eq!(
        run.status.code(),
        Some(1),
        "a --key that cannot load is a load error, exit 1"
    );
    assert!(
        !out.join("adjudications.md").exists(),
        "fail-fast, all-or-nothing: no adjudications.md when --key cannot load"
    );
    assert!(
        !out.join("solution").exists(),
        "no partial solution/ tree is left behind either"
    );

    let _ = std::fs::remove_dir_all(&out);
}

/// A malformed `--base-version` is a load/parse-class failure (exit 1), not a
/// silent success.
#[test]
fn export_with_bad_base_version_fails() {
    let out = temp_out("badver");
    let run = crucible()
        .arg("export")
        .arg("--labels")
        .arg(fixture("export-queue.json"))
        .arg("--out")
        .arg(&out)
        .arg("--arena")
        .arg("pr-review-v0")
        .arg("--task")
        .arg("py-file-cache")
        .arg("--base-version")
        .arg("not-a-version")
        .output()
        .expect("crucible binary runs");
    assert_eq!(
        run.status.code(),
        Some(1),
        "a malformed base version is a load error, exit 1"
    );

    let _ = std::fs::remove_dir_all(&out);
}
