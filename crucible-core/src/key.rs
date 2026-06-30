//! Daedalus answer-key types: the ground truth a review is scored against.
//!
//! A Daedalus task carries the key in **two** shapes, and Crucible models both:
//!
//! - [`AnswerKey`] — `solution/findings.json`, shaped
//!   `{ "findings": [{ file, line, category, severity?, description }] }`: the
//!   human-readable point oracle.
//! - [`ExpectedKey`] — `tests/expected.json`, shaped `{ "defects": [{ id, file,
//!   line_start, line_end, category, severity?, note? }] }`: the **line-span** key
//!   `daedalus-score` actually scores against (daedalus
//!   `crates/daedalus-core/src/score.rs`, `load_expected`). This is the file a
//!   re-score reads, so an accepted finding must be written *here* to count as a
//!   true positive rather than a false positive.

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
    /// it; it is carried for display and downstream judgment only. `""` is also
    /// skipped on the wire so a severity-less real key — which `crucible export
    /// --key` re-emits as a Daedalus oracle — round-trips byte-faithfully
    /// (absent in stays absent out) rather than gaining a spurious
    /// `"severity": ""`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub severity: String,
    /// Human description of the expected finding.
    pub description: String,
    /// For a **candidate** row projected from a Cerberus review, the source
    /// [`Finding`](crate::Finding)'s artifact-stable id (e.g. `F1`), threaded by
    /// [`to_key_findings`](crate::to_key_findings) so a downstream adjudication
    /// [`Label`](crate::Label) traces back to the finding it judges. Answer-key
    /// rows loaded from a Daedalus `solution/findings.json` carry **no**
    /// per-finding id, so this is `None` for them and absent on the wire:
    /// `#[serde(default, skip_serializing_if = "Option::is_none")]` keeps a real
    /// key — which never names an id — parsing and round-tripping unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

/// A Daedalus *scorer* answer key: a `tests/expected.json`, shaped
/// `{ "defects": [{ id, file, line_start, line_end, category, severity?, note? }] }`.
///
/// This is the key `daedalus-score` reads (daedalus `score.rs`, `load_expected`):
/// a finding scores a hit when its `file` and `category` equal a defect's and its
/// `line` falls inside `[line_start, line_end]`; a finding matching no defect is a
/// false positive (`reward = max(0, recall − 0.2·FP)`). It is **distinct** from
/// [`AnswerKey`] (`solution/findings.json`, the human point oracle): only this
/// span key feeds the scorer, so [`crate::export`] writes *both* on an ACCEPT.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedKey {
    /// The seeded defects the scorer matches a review against. Defaults to empty.
    #[serde(default)]
    pub defects: Vec<Defect>,
}

impl ExpectedKey {
    /// Deserialize a scorer key from a JSON string.
    pub fn from_json_str(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// Load and deserialize a scorer key from a JSON file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| Error::read(path, e))?;
        serde_json::from_slice(&bytes).map_err(|e| Error::parse(path, e))
    }
}

/// One seeded defect in an [`ExpectedKey`] (`tests/expected.json`).
///
/// Field-faithful to the shape `daedalus-score`'s `load_expected` deserializes:
/// an `id` it reports on a hit, the `file`+`category` it matches on, the
/// `[line_start, line_end]` span a finding's line must fall inside, an optional
/// `severity` floor (the scorer ranks `blocking` > `serious` > `minor`), and a
/// free-text `note`. The scorer **ignores** `note`; it carries the human
/// rationale — the `solution/findings.json` `description`'s analogue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Defect {
    /// Stable id the scorer reports on a hit; unique within the file.
    pub id: String,
    /// Repo-relative file the defect lives in.
    pub file: String,
    /// First line of the defect's span (inclusive).
    pub line_start: u32,
    /// Last line of the defect's span (inclusive).
    pub line_end: u32,
    /// Finding category the scorer matches on, e.g. `security`.
    pub category: String,
    /// Optional severity floor. Absent — the common case, and what Crucible
    /// writes (see [`crate::extended_expected_key`]) — means the scorer ignores
    /// severity for this defect. Skipped on the wire when absent so a real
    /// severity-less key round-trips byte-faithfully.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// Free-text rationale. The scorer never reads it; skipped on the wire when
    /// empty so a note-less defect does not gain a spurious `"note": ""`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl Defect {
    /// Project this defect onto a [`KeyFinding`] so a `tests/expected.json` can be
    /// read by the same [`mod@crate::grade`] matcher as a `solution/findings.json`.
    ///
    /// `line` takes the span's `line_start` and `note` carries over as the
    /// description. This is an **approximation**: Crucible's pre-grader matcher is
    /// point-and-tolerance (`±LINE_TOLERANCE`), not span-aware, so a defect's span
    /// is represented by its start. It is enough for `crucible grade` to *read*
    /// and partition the scorer key (the alternative being a silent zero-row
    /// grade); the authoritative span match stays with `daedalus-score`.
    pub fn to_key_finding(&self) -> KeyFinding {
        KeyFinding {
            file: self.file.clone(),
            line: self.line_start,
            category: self.category.clone(),
            severity: self.severity.clone().unwrap_or_default(),
            description: self.note.clone(),
            source_id: None,
        }
    }
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
        assert!(
            kf.source_id.is_none(),
            "a Daedalus key row carries no per-finding id"
        );
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
        assert!(
            key.findings.iter().all(|f| f.source_id.is_none()),
            "real key rows never carry a source id"
        );
    }

    #[test]
    fn source_id_round_trips_for_candidates_and_is_absent_for_keys() {
        // A candidate row (projected from a review) carries its source finding id
        // and emits it on the wire; a key row leaves it None and omits it, so the
        // two share one type without the key ever growing an `id` field.
        let candidate = KeyFinding {
            file: "src/x.rs".to_string(),
            line: 7,
            category: "security".to_string(),
            severity: "blocking".to_string(),
            description: "d".to_string(),
            source_id: Some("F1".to_string()),
        };
        let json = serde_json::to_string(&candidate).unwrap();
        assert!(json.contains(r#""source_id":"F1""#), "{json}");
        assert_eq!(
            serde_json::from_str::<KeyFinding>(&json).unwrap(),
            candidate
        );

        let key: KeyFinding = serde_json::from_str(
            r#"{"file":"src/x.rs","line":7,"category":"security","description":"d"}"#,
        )
        .unwrap();
        assert!(key.source_id.is_none());
        assert!(
            !serde_json::to_string(&key).unwrap().contains("source_id"),
            "an absent source id is skipped on the wire"
        );
    }

    // ---- ExpectedKey / Defect (tests/expected.json) -----------------------

    #[test]
    fn expected_key_parses_daedalus_defects_shape() {
        // Verbatim shape of pr-review-v0/py-auth-sqli/tests/expected.json — the
        // span key daedalus-score reads (id + line_start/line_end + note, no
        // severity). It must parse, severity defaulting to None.
        let json = r#"{
            "defects": [
                {
                    "id": "sqli",
                    "file": "app/auth.py",
                    "line_start": 8,
                    "line_end": 12,
                    "category": "security",
                    "note": "Query is built by interpolating the email into the SQL string."
                },
                {
                    "id": "swallowed-db-errors",
                    "file": "app/auth.py",
                    "line_start": 13,
                    "line_end": 14,
                    "category": "error-handling"
                }
            ]
        }"#;
        let key = ExpectedKey::from_json_str(json).expect("real expected.json must parse");
        assert_eq!(key.defects.len(), 2);
        let d0 = &key.defects[0];
        assert_eq!(d0.id, "sqli");
        assert_eq!(d0.file, "app/auth.py");
        assert_eq!((d0.line_start, d0.line_end), (8, 12));
        assert_eq!(d0.category, "security");
        assert!(d0.note.contains("interpolating"));
        assert!(d0.severity.is_none(), "a note-only defect has no severity");
        assert_eq!(key.defects[1].note, "", "an absent note defaults to empty");
    }

    #[test]
    fn expected_key_defaults_to_empty() {
        let key = ExpectedKey::from_json_str("{}").unwrap();
        assert!(key.defects.is_empty());
    }

    #[test]
    fn defect_projects_to_key_finding_at_line_start() {
        let defect = Defect {
            id: "d1".to_string(),
            file: "app/auth.py".to_string(),
            line_start: 8,
            line_end: 12,
            category: "security".to_string(),
            severity: Some("blocking".to_string()),
            note: "sql injection".to_string(),
        };
        let kf = defect.to_key_finding();
        assert_eq!(kf.file, "app/auth.py");
        assert_eq!(kf.line, 8, "the span collapses to its start");
        assert_eq!(kf.category, "security");
        assert_eq!(kf.severity, "blocking", "severity carries over");
        assert_eq!(kf.description, "sql injection", "note becomes description");
        assert!(kf.source_id.is_none());
    }

    #[test]
    fn defect_skips_absent_severity_and_note_on_the_wire() {
        // The shape Crucible writes on an ACCEPT: no severity, a note. The output
        // must carry `note` but no `"severity"` key, matching the real file.
        let defect = Defect {
            id: "resource-leak-app-auth-py-6".to_string(),
            file: "app/auth.py".to_string(),
            line_start: 6,
            line_end: 6,
            category: "resource-leak".to_string(),
            severity: None,
            note: "connection never closed".to_string(),
        };
        let json = serde_json::to_string(&defect).unwrap();
        assert!(
            !json.contains("severity"),
            "absent severity is skipped: {json}"
        );
        assert!(
            json.contains(r#""note":"connection never closed""#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Defect>(&json).unwrap(), defect);
    }
}
