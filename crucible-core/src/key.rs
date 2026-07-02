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

/// The deterministic grade of candidate findings against an [`ExpectedKey`]'s
/// defect spans (backlog 013): a candidate hits a defect when its `file` and
/// `category` equal the defect's, its `line` falls inside
/// `[line_start, line_end]`, and it clears the defect's optional severity
/// floor. This is the **span-aware** match — distinct from
/// [`crate::key_match`]'s point-and-tolerance approximation
/// ([`Defect::to_key_finding`] documents why that one exists) — the same
/// semantics `daedalus-score` implements, extracted here so Crucible's
/// key-recall runner and Threshold/Daedalus share one scorer by construction
/// instead of by prose parity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanGrade {
    /// Ids of defects a candidate finding matched.
    pub matched_ids: Vec<String>,
    /// Ids of defects no candidate finding matched.
    pub missed_ids: Vec<String>,
    /// Candidate findings that matched no defect.
    pub false_positives: u64,
}

/// Grade candidate findings against an expected key's defect spans.
///
/// Greedy and order-sensitive, matching [`mod@crate::grade`]'s matcher: each
/// candidate, in order, claims the first still-unclaimed defect it matches on
/// file, category, span, and severity floor. A candidate that claims no
/// defect counts as a false positive; a defect no candidate claims is missed.
pub fn score_against_expected_key(findings: &[KeyFinding], expected: &ExpectedKey) -> SpanGrade {
    let mut matched_flags = vec![false; expected.defects.len()];
    let mut matched_ids = Vec::new();
    let mut false_positives = 0u64;

    for finding in findings {
        let hit = expected
            .defects
            .iter()
            .enumerate()
            .position(|(i, defect)| !matched_flags[i] && defect_matches(finding, defect));
        match hit {
            Some(i) => {
                matched_flags[i] = true;
                matched_ids.push(expected.defects[i].id.clone());
            }
            None => false_positives += 1,
        }
    }

    let missed_ids = expected
        .defects
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_flags[*i])
        .map(|(_, defect)| defect.id.clone())
        .collect();

    SpanGrade {
        matched_ids,
        missed_ids,
        false_positives,
    }
}

/// Whether a candidate finding hits a defect: same file, same category, the
/// finding's line falls inside the defect's `[line_start, line_end]` span, and
/// the finding clears the defect's optional severity floor.
fn defect_matches(finding: &KeyFinding, defect: &Defect) -> bool {
    finding.file == defect.file
        && finding.category == defect.category
        && defect.line_start <= finding.line
        && finding.line <= defect.line_end
        && severity_matches(finding.severity.as_str(), defect.severity.as_deref())
}

/// Whether a candidate's severity clears an optional severity floor.
///
/// An absent floor (`expected: None`, the common case per [`Defect::severity`])
/// always clears. A present floor requires both labels to rank
/// (`blocking` > `serious` > `minor`, an unrecognized label ranks as neither)
/// and the candidate to be at least as severe as the floor.
fn severity_matches(candidate: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    match (severity_rank(candidate), severity_rank(expected)) {
        (Some(candidate), Some(expected)) => candidate <= expected,
        _ => false,
    }
}

/// Daedalus's severity vocabulary, ranked most to least severe. `None` for an
/// unrecognized label — a floor or candidate outside this vocabulary can never
/// satisfy [`severity_matches`], rather than silently defaulting to a rank.
fn severity_rank(label: &str) -> Option<u8> {
    match label {
        "blocking" => Some(0),
        "serious" => Some(1),
        "minor" => Some(2),
        _ => None,
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

    fn defect(id: &str, file: &str, span: (u32, u32), category: &str) -> Defect {
        Defect {
            id: id.to_string(),
            file: file.to_string(),
            line_start: span.0,
            line_end: span.1,
            category: category.to_string(),
            severity: None,
            note: String::new(),
        }
    }

    fn defect_with_severity(
        id: &str,
        file: &str,
        span: (u32, u32),
        category: &str,
        severity: &str,
    ) -> Defect {
        Defect {
            severity: Some(severity.to_string()),
            ..defect(id, file, span, category)
        }
    }

    fn finding(file: &str, line: u32, category: &str) -> KeyFinding {
        KeyFinding {
            file: file.to_string(),
            line,
            category: category.to_string(),
            severity: String::new(),
            description: String::new(),
            source_id: None,
        }
    }

    fn finding_with_severity(file: &str, line: u32, category: &str, severity: &str) -> KeyFinding {
        KeyFinding {
            severity: severity.to_string(),
            ..finding(file, line, category)
        }
    }

    #[test]
    fn span_grade_matches_a_line_anywhere_inside_the_defect_span() {
        let expected = ExpectedKey {
            defects: vec![defect("d1", "src/lib.rs", (10, 20), "correctness")],
        };
        for line in [10, 15, 20] {
            let grade = score_against_expected_key(
                &[finding("src/lib.rs", line, "correctness")],
                &expected,
            );
            assert_eq!(
                grade,
                SpanGrade {
                    matched_ids: vec!["d1".to_string()],
                    missed_ids: Vec::new(),
                    false_positives: 0,
                },
                "line {line} is inside [10, 20]"
            );
        }
    }

    #[test]
    fn span_grade_misses_a_line_just_outside_the_defect_span() {
        let expected = ExpectedKey {
            defects: vec![defect("d1", "src/lib.rs", (10, 20), "correctness")],
        };
        for line in [9, 21] {
            let grade = score_against_expected_key(
                &[finding("src/lib.rs", line, "correctness")],
                &expected,
            );
            assert_eq!(
                grade,
                SpanGrade {
                    matched_ids: Vec::new(),
                    missed_ids: vec!["d1".to_string()],
                    false_positives: 1,
                },
                "line {line} is outside [10, 20]: unlike crate::key_match, this scorer has no ±tolerance"
            );
        }
    }

    #[test]
    fn span_grade_requires_exact_category_agreement() {
        let expected = ExpectedKey {
            defects: vec![defect("d1", "src/lib.rs", (10, 20), "correctness")],
        };
        let grade = score_against_expected_key(&[finding("src/lib.rs", 12, "security")], &expected);
        assert_eq!(
            grade,
            SpanGrade {
                matched_ids: Vec::new(),
                missed_ids: vec!["d1".to_string()],
                false_positives: 1,
            },
            "in-span but wrong category is a miss plus a false positive, not a match"
        );
    }

    #[test]
    fn span_grade_severity_floor_accepts_at_least_as_severe() {
        let expected = ExpectedKey {
            defects: vec![defect_with_severity(
                "d1",
                "src/lib.rs",
                (10, 20),
                "correctness",
                "serious",
            )],
        };
        for candidate_severity in ["blocking", "serious"] {
            let grade = score_against_expected_key(
                &[finding_with_severity(
                    "src/lib.rs",
                    12,
                    "correctness",
                    candidate_severity,
                )],
                &expected,
            );
            assert_eq!(
                grade.matched_ids,
                vec!["d1".to_string()],
                "{candidate_severity} clears a serious floor"
            );
        }
    }

    #[test]
    fn span_grade_severity_floor_rejects_less_severe_or_unranked() {
        let expected = ExpectedKey {
            defects: vec![defect_with_severity(
                "d1",
                "src/lib.rs",
                (10, 20),
                "correctness",
                "serious",
            )],
        };
        for candidate_severity in ["minor", "unranked-label", ""] {
            let grade = score_against_expected_key(
                &[finding_with_severity(
                    "src/lib.rs",
                    12,
                    "correctness",
                    candidate_severity,
                )],
                &expected,
            );
            assert_eq!(
                grade.false_positives, 1,
                "{candidate_severity:?} does not clear a serious floor"
            );
        }
    }

    #[test]
    fn span_grade_no_severity_floor_accepts_any_candidate_severity() {
        let expected = ExpectedKey {
            defects: vec![defect("d1", "src/lib.rs", (10, 20), "correctness")],
        };
        let grade = score_against_expected_key(
            &[finding_with_severity(
                "src/lib.rs",
                12,
                "correctness",
                "minor",
            )],
            &expected,
        );
        assert_eq!(
            grade.matched_ids,
            vec!["d1".to_string()],
            "an absent floor accepts any candidate severity, even the least severe"
        );
    }

    #[test]
    fn span_grade_greedy_matching_claims_the_first_unclaimed_defect() {
        let expected = ExpectedKey {
            defects: vec![
                defect("d1", "src/lib.rs", (10, 20), "correctness"),
                defect("d2", "src/lib.rs", (10, 20), "correctness"),
            ],
        };
        let grade = score_against_expected_key(
            &[
                finding("src/lib.rs", 12, "correctness"),
                finding("src/lib.rs", 15, "correctness"),
            ],
            &expected,
        );
        assert_eq!(grade.matched_ids, vec!["d1".to_string(), "d2".to_string()]);
        assert_eq!(grade.false_positives, 0);
    }

    #[test]
    fn span_grade_extra_candidate_beyond_available_defects_is_a_false_positive() {
        let expected = ExpectedKey {
            defects: vec![defect("d1", "src/lib.rs", (10, 20), "correctness")],
        };
        let grade = score_against_expected_key(
            &[
                finding("src/lib.rs", 12, "correctness"),
                finding("src/lib.rs", 14, "correctness"),
            ],
            &expected,
        );
        assert_eq!(grade.matched_ids, vec!["d1".to_string()]);
        assert_eq!(grade.missed_ids, Vec::<String>::new());
        assert_eq!(grade.false_positives, 1);
    }
}
