//! Daedalus answer-key types: the ground truth a review is scored against.
//!
//! Source of truth: a Daedalus arena `solution/findings.json`, shaped
//! `{ "findings": [{ file, line, category, severity, description }] }`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// A Daedalus answer key: the expected findings for one task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerKey {
    /// The findings a correct review is expected to surface. Defaults to empty.
    #[serde(default)]
    pub findings: Vec<KeyFinding>,
}

impl AnswerKey {
    /// Deserialize a key from a JSON string.
    pub fn from_json_str(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Load and deserialize a key from a JSON file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| Error::read(path, e))?;
        serde_json::from_slice(&bytes).map_err(|e| Error::parse(path, e))
    }
}

/// One expected finding in the answer key.
///
/// `severity` is Daedalus's own vocabulary (e.g. `blocking`), distinct from
/// Cerberus [`Severity`](crate::Severity); it stays a free-form string until a
/// mapping is defined by the matcher in a later step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyFinding {
    /// Repo-relative file the finding lives in.
    pub file: String,
    /// Line the finding is anchored to.
    pub line: u32,
    /// Finding category, e.g. `runtime-crash`.
    pub category: String,
    /// Daedalus severity vocabulary, e.g. `blocking`. **Optional**: roughly half
    /// of real Daedalus `solution/findings.json` keys omit it, so it defaults to
    /// `""` when absent rather than hard-erroring the whole key. The matcher
    /// ([`key_match`](crate::key_match)) and [`dedup`](crate::dedup) never read
    /// it; it is carried for display and downstream judgment only.
    #[serde(default)]
    pub severity: String,
    /// Human description of the expected finding.
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_deserializes_daedalus_shape() {
        // Verbatim shape of daedalus .../runtime-crash/solution/findings.json.
        let json = r#"{
            "findings": [
                {
                    "file": "src/ingest.py",
                    "line": 88,
                    "category": "runtime-crash",
                    "severity": "blocking",
                    "description": "The new direct payload lookup raises KeyError for normal ping/dry-run events without payload."
                }
            ]
        }"#;
        let key = AnswerKey::from_json_str(json).unwrap();
        assert_eq!(key.findings.len(), 1);
        let kf = &key.findings[0];
        assert_eq!(kf.file, "src/ingest.py");
        assert_eq!(kf.line, 88);
        assert_eq!(kf.category, "runtime-crash");
        assert_eq!(kf.severity, "blocking");
        assert!(kf.description.contains("KeyError"));
    }

    #[test]
    fn key_defaults_to_empty_findings() {
        let key = AnswerKey::from_json_str("{}").unwrap();
        assert!(key.findings.is_empty());
    }

    #[test]
    fn key_parses_real_severity_less_findings() {
        // Roughly half of real Daedalus solution keys omit `severity`. Verbatim
        // shape of pr-review-v0/.../py-auth-sqli/solution/findings.json (one of
        // the severity-less majority); it must parse, with severity defaulting
        // to "" rather than the whole key failing to load.
        let json = r#"{
            "findings": [
                {
                    "file": "app/auth.py",
                    "line": 10,
                    "category": "security",
                    "description": "The SQL query interpolates the user-supplied email directly into the string, allowing SQL injection. Use a parameterized query as the previous code did."
                },
                {
                    "file": "app/auth.py",
                    "line": 13,
                    "category": "error-handling",
                    "description": "The broad except swallows all database errors and returns None, so an outage is indistinguishable from invalid credentials and the real error is lost."
                }
            ]
        }"#;
        let key = AnswerKey::from_json_str(json).expect("severity-less key must parse");
        assert_eq!(key.findings.len(), 2);
        assert_eq!(
            key.findings[0].severity, "",
            "absent severity defaults to empty"
        );
        assert_eq!(key.findings[0].category, "security");
        assert_eq!(key.findings[1].category, "error-handling");
    }
}
