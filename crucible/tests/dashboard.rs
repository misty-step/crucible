//! End-to-end CLI test for `crucible dashboard`, driving the built binary as a
//! subprocess (no `assert_cmd`; Cargo hands the path in `CARGO_BIN_EXE_crucible`).
//!
//! The corpus is a small synthetic arenas/runs tree built on disk at runtime —
//! the same hermetic, no-committed-fixtures idiom the `crucible_core::dashboard`
//! unit tests use — so the test ships no raw run records and depends on no local
//! Daedalus checkout. It exercises the whole command path: ingest → measure →
//! write, then asserts both artifacts exist, the HTML carries the real arena,
//! config, and verdict surfaces, and `data.json` is the stable model that pins the
//! noise-floor verdicts the page renders.
//!
//! The three configs are shaped to force *both* tested verdict branches on enough
//! shared tasks to clear the power floor: an `oracle` that fully solves all eight
//! tasks, a `probe-oneshot` that fully solves seven of eight, and a `null` that
//! solves none. The oracle's one-task edge over the probe sits inside the noise
//! floor (McNemar p = 1, and the seed-stable paired-bootstrap envelope straddles
//! 0), while the probe's seven-task edge over null is a directional signal — so a
//! correct board emits one `signal` and one `inside_noise_floor` verdict, and the
//! test checks for each. Reading the verdict off a seed *envelope* makes both
//! decisions stable across seeds, not merely reproducible under one.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn crucible() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crucible"))
}

/// A fresh, unique scratch root under the system temp dir.
fn temp_root(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "crucible-dashboard-{}-{tag}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp root");
    dir
}

/// Write `contents` to `rel` under `root`, creating parent dirs.
fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("rel has a parent")).expect("create parents");
    std::fs::write(path, contents).expect("write fixture file");
}

/// One `trials.jsonl` line in the real Daedalus shape (extra real fields the model
/// ignores are omitted; the loader fills the rest from defaults).
fn trial(hash: &str, id: &str, kind: &str, task: &str, trial: i64, reward: f64) -> String {
    format!(
        r#"{{"run_id":"{id}-{task}-t{trial}","arena_id":"arena-x","arena_version":"0.1.0","task_id":"{task}","trial":{trial},"candidate_id":"{id}","candidate_kind":"{kind}","composition_hash":"{hash}","model":null,"cost_usd":null,"error":null,"wall_ms":12,"reward":{reward},"recall":{reward},"matched":[],"false_positives":0,"expected_defects":1,"scorer_error":null}}"#
    )
}

/// Build the synthetic arenas/runs tree and return `(arenas, runs)`.
fn build_corpus(root: &Path) -> (PathBuf, PathBuf) {
    // One real answer key, so a task renders a non-zero defect count.
    write(
        root,
        "arenas/arena-x/tasks/t1/tests/expected.json",
        r#"{"defects":[{"id":"d1","file":"f.py","line_start":1,"line_end":2,"category":"security","note":"seeded"}]}"#,
    );

    // run-a: three configs over EIGHT tasks — above the power floor, so the rank
    // gaps are testable rather than refused as underpowered.
    let mut lines = Vec::new();
    for i in 1..=8 {
        let task = format!("t{i}");
        // oracle fully solves every task; null solves none.
        lines.push(trial("oracle", "oracle", "oracle", &task, 1, 1.0));
        lines.push(trial("null", "null", "null", &task, 1, 0.0));
        // probe-oneshot fully solves 7 of 8 (misses t8) — a hair behind oracle,
        // a chasm ahead of null.
        let probe_reward = if i == 8 { 0.0 } else { 1.0 };
        lines.push(trial(
            "probe1",
            "probe-oneshot",
            "oneshot",
            &task,
            1,
            probe_reward,
        ));
    }
    // A malformed line proves the skip count is surfaced, not fatal.
    lines.push("this is not json".to_string());
    write(
        root,
        "runs/run-a/trials.jsonl",
        &format!("{}\n", lines.join("\n")),
    );

    // run-b: more oracle trials on t1 — pooled into the same config across runs,
    // and a second run receipt feeding the eval.
    write(
        root,
        "runs/run-b/trials.jsonl",
        &format!("{}\n", trial("oracle", "oracle", "oracle", "t1", 2, 1.0)),
    );

    (root.join("arenas"), root.join("runs"))
}

/// The headline test: the command exits 0, writes both artifacts, and the HTML +
/// `data.json` carry the real arenas, configs, intervals, and noise-floor
/// verdicts.
#[test]
fn dashboard_renders_real_surfaces_and_a_stable_model() {
    let root = temp_root("render");
    let (arenas, runs) = build_corpus(&root);
    let out = root.join("out");

    let run = crucible()
        .arg("dashboard")
        .arg("--arenas")
        .arg(&arenas)
        .arg("--runs")
        .arg(&runs)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("crucible binary runs");
    assert!(
        run.status.success(),
        "dashboard must exit 0; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let html_path = out.join("index.html");
    let data_path = out.join("data.json");
    assert!(html_path.exists(), "index.html must be written");
    assert!(data_path.exists(), "data.json must be written");

    // ----- HTML carries the real surfaces -----
    let html = std::fs::read_to_string(&html_path).expect("read index.html");
    assert!(html.starts_with("<!doctype html>"), "a real HTML document");
    assert!(
        html.contains("name=\"viewport\""),
        "phone-first: a viewport meta tag"
    );
    for marker in [
        "Crucible",    // header
        "arena-x",     // the real arena id
        "0.1.0",       // arena version, rendered prominently
        "Leaderboard", // the centerpiece
        "oracle",      // real config ids appear
        "probe-oneshot",
        "null",
        "run-a",                   // the run drill-down lists the directory
        "bootstrap",               // reward interval method labeled
        "Wilson",                  // solve interval method labeled
        "1.00",                    // a real reward number
        "1 defect",                // the seeded task key surfaced
        "stronger than runner-up", // the signal verdict surface
        "inside noise floor",      // the noise-floor verdict surface
    ] {
        assert!(
            html.contains(marker),
            "rendered HTML is missing the marker {marker:?}"
        );
    }

    // ----- data.json is the stable model the page renders -----
    let data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&data_path).expect("read data.json"))
            .expect("data.json is valid JSON");
    assert_eq!(
        data["schema_version"], "crucible.dashboard.v1",
        "data.json carries a stable schema id"
    );
    assert!(
        !data["dataset"]["evals"]
            .as_array()
            .expect("evals array")
            .is_empty(),
        "the dataset ingested at least one eval"
    );
    assert_eq!(
        data["dataset"]["skipped"], 1,
        "the one malformed line is surfaced as skipped, not fatal"
    );
    assert!(
        !data["run_details"]
            .as_array()
            .expect("run_details array")
            .is_empty(),
        "the run drill-down model is populated"
    );

    // The leaderboard pins one of each verdict: probe-oneshot ≫ null is a
    // directional signal, oracle ≈ probe-oneshot sits inside the noise floor.
    let groups = data["leaderboard"]["groups"]
        .as_array()
        .expect("leaderboard groups");
    let entries = groups[0]["entries"].as_array().expect("group entries");
    assert_eq!(
        entries.len(),
        3,
        "three configs ranked: oracle, probe, null"
    );
    assert_eq!(entries[0]["id"], "oracle", "the all-solver ranks #1");

    // The verdict is now internally tagged on `kind`, and a signal carries the
    // direction it claims.
    let kinds: Vec<&str> = entries
        .iter()
        .filter_map(|e| e["vs_next"]["verdict"]["kind"].as_str())
        .collect();
    assert!(
        kinds.contains(&"signal"),
        "a clear gap (probe ≫ null) must be a signal: {kinds:?}"
    );
    assert!(
        kinds.contains(&"inside_noise_floor"),
        "a one-task gap (oracle ≈ probe) must be refused: {kinds:?}"
    );
    // The signal names the higher-ranked config as stronger — a directional claim.
    let signal = entries
        .iter()
        .find(|e| e["vs_next"]["verdict"]["kind"] == "signal")
        .expect("a signal entry");
    assert_eq!(
        signal["vs_next"]["verdict"]["stronger"], "higher",
        "the probe ≫ null signal must name the higher-ranked config"
    );

    // Reward-mean interval brackets its point and is a bootstrap; solve rate is
    // Wilson — the two methods the page labels.
    let lead_reward = &entries[0]["reward_mean"];
    assert_eq!(lead_reward["method"], "bootstrap");
    let point = lead_reward["point"].as_f64().expect("reward point");
    let lower = lead_reward["lower"].as_f64().expect("reward lower");
    let upper = lead_reward["upper"].as_f64().expect("reward upper");
    assert!(
        lower <= point && point <= upper,
        "interval [{lower}, {upper}] must bracket {point}"
    );
    assert_eq!(entries[0]["solve_rate"]["method"], "wilson");

    let _ = std::fs::remove_dir_all(&root);
}

/// A `--runs` path that is not a directory is an operational failure (exit 1),
/// surfaced up front rather than silently producing an empty dashboard.
#[test]
fn dashboard_with_a_nondirectory_runs_path_fails() {
    let root = temp_root("badruns");
    let out = root.join("out");

    let run = crucible()
        .arg("dashboard")
        .arg("--arenas")
        .arg(root.join("arenas"))
        .arg("--runs")
        .arg(root.join("does-not-exist"))
        .arg("--out")
        .arg(&out)
        .output()
        .expect("crucible binary runs");
    assert_eq!(
        run.status.code(),
        Some(1),
        "a missing runs dir is a load error, exit 1"
    );
    assert!(
        !out.join("index.html").exists(),
        "no dashboard is written when the runs path is invalid"
    );

    let _ = std::fs::remove_dir_all(&root);
}
