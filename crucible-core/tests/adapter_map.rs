//! Integration test: the real Cerberus artifact maps through the adapter into
//! well-formed Daedalus answer-key rows.
//!
//! Exercises the file-loading entry point [`findings_from_artifact`] against the
//! verbatim fixture copy (`cerberus/evidence/self-review-001/artifact.json`,
//! also used by `fixture_parse.rs`), then projects with [`to_key_findings`] and
//! asserts the projection is well-formed: a non-empty `file` where the finding
//! has an anchor, the anchored line, a pass-through category, the mapped
//! severity, and a description carrying the finding's headline and body.

use std::path::{Path, PathBuf};

use crucible_core::adapter::{findings_from_artifact, to_key_findings};

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cerberus-artifact.json")
}

#[test]
fn real_artifact_maps_to_well_formed_key_findings() {
    let findings =
        findings_from_artifact(fixture_path()).expect("real artifact must load and parse");
    assert_eq!(findings.len(), 1, "fixture has exactly one finding");

    let keys = to_key_findings(&findings);
    assert_eq!(keys.len(), findings.len(), "one key row per finding");

    let k = &keys[0];
    // file/line come from F1's inline anchor (src/harness.rs:349).
    assert!(!k.file.is_empty(), "file non-empty where an anchor exists");
    assert_eq!(k.file, "src/harness.rs", "file from inline anchor path");
    assert_eq!(k.line, 349, "line from inline anchor");
    // category passes through; Cerberus `minor` maps to Daedalus `minor`.
    assert_eq!(k.category, "security");
    assert_eq!(k.severity, "minor");
    // description carries both the finding's headline and its rationale body.
    assert!(
        k.description.contains("grants web tools unconditionally"),
        "description carries the finding title"
    );
    assert!(
        k.description.contains("external_research"),
        "description carries the finding body"
    );
}
