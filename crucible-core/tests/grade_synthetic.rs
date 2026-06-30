//! Integration test: grade the real Cerberus artifact's findings against a
//! synthetic answer key, exercising the whole deterministic pipeline —
//! [`schema_valid`] filter -> [`to_key_findings`] adapter -> [`dedup`] ->
//! [`grade`]. The fixture is the verbatim copy of
//! `cerberus/evidence/self-review-001/artifact.json` (one finding: a `security`
//! finding anchored at `src/harness.rs:349`), also used by the other suites.

use std::path::{Path, PathBuf};

use crucible_core::adapter::{findings_from_artifact, to_key_findings};
use crucible_core::grade::{dedup, grade, schema_valid};
use crucible_core::KeyFinding;

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cerberus-artifact.json")
}

/// The candidate side: load the real artifact, keep only schema-valid findings,
/// project them onto answer-key rows. The fixture's single finding is valid.
fn candidate_rows() -> Vec<KeyFinding> {
    let findings = findings_from_artifact(fixture_path()).expect("real artifact must load");
    assert_eq!(findings.len(), 1, "fixture has one finding");
    assert!(
        schema_valid(&findings[0]),
        "the fixture finding is well-formed"
    );

    let valid: Vec<_> = findings.into_iter().filter(schema_valid).collect();
    to_key_findings(&valid)
}

#[test]
fn real_candidate_matches_synthetic_key_and_reports_a_missed_row() {
    let cand = candidate_rows();

    // A key with: a row the candidate satisfies (line +1 within tolerance,
    // category differing only in case), a duplicate of it (dedup must remove),
    // and a row no candidate finds (must land in `missed`).
    let key = dedup(vec![
        key_row("src/harness.rs", 350, "Security"),
        key_row("src/harness.rs", 350, "Security"), // exact duplicate
        key_row("src/prompt.rs", 42, "correctness"), // unfound by the review
    ]);
    assert_eq!(key.len(), 2, "dedup collapses the duplicate key row");

    let result = grade(&cand, &key);

    assert_eq!(
        result.matched.len(),
        1,
        "the harness finding matches its key row"
    );
    assert_eq!(result.matched[0].candidate.file, "src/harness.rs");
    assert_eq!(result.matched[0].key.line, 350);

    assert!(result.disputed.is_empty(), "the only candidate matched");

    assert_eq!(result.missed.len(), 1, "the prompt.rs key row was missed");
    assert_eq!(result.missed[0].file, "src/prompt.rs");
}

#[test]
fn candidate_off_by_more_than_tolerance_is_disputed_and_the_key_is_missed() {
    let cand = candidate_rows();

    // Same file and category, but the key line is far from the finding's 349,
    // so neither side resolves: candidate disputed, key missed.
    let key = vec![key_row("src/harness.rs", 400, "security")];

    let result = grade(&cand, &key);

    assert!(
        result.matched.is_empty(),
        "line 349 vs 400 is outside tolerance"
    );
    assert_eq!(
        result.disputed.len(),
        1,
        "the unmatched finding is disputed"
    );
    assert_eq!(result.disputed[0].file, "src/harness.rs");
    assert_eq!(result.missed, key, "the unfound key row is missed");
}

fn key_row(file: &str, line: u32, category: &str) -> KeyFinding {
    KeyFinding {
        file: file.to_string(),
        line,
        category: category.to_string(),
        severity: "blocking".to_string(),
        description: "expected finding".to_string(),
    }
}
