//! Integration test: the real Cerberus artifact deserializes into the core
//! types. The fixture is copied verbatim from
//! `cerberus/evidence/self-review-001/artifact.json` so the test is hermetic
//! and does not reach into a sibling repo.

use crucible_core::{AnchorKind, ReviewArtifact, Severity};

const FIXTURE: &str = include_str!("fixtures/cerberus-artifact.json");

#[test]
fn real_cerberus_artifact_parses() {
    let artifact = ReviewArtifact::from_json_str(FIXTURE).expect("real artifact must deserialize");

    assert_eq!(artifact.schema_version, "cerberus.review_artifact.v1");
    assert_eq!(
        artifact.findings.len(),
        1,
        "fixture has exactly one finding"
    );

    let f = &artifact.findings[0];
    assert_eq!(f.id, "F1");
    assert_eq!(f.severity, Severity::Minor);
    assert_eq!(f.category, "security");
    assert!(
        (0.0..=1.0).contains(&f.confidence),
        "confidence in unit range"
    );

    assert_eq!(f.anchors.len(), 1, "F1 has one anchor");
    let a = &f.anchors[0];
    assert_eq!(a.kind, AnchorKind::Inline);
    assert_eq!(a.path.as_deref(), Some("src/harness.rs"));
    assert_eq!(a.line, Some(349));
    assert_eq!(a.start_line, Some(343));
    assert_eq!(a.end_line, Some(352));
}
