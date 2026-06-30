//! Deterministic core for Crucible's code-review eval.
//!
//! Four type domains plus the adapter that bridges two of them:
//!
//! - [`artifact`] — a Cerberus `ReviewArtifact` (the review under evaluation),
//!   mirrored just deeply enough to read findings and their anchors.
//! - [`key`] — a Daedalus answer key (`solution/findings.json`): the ground
//!   truth a review is scored against.
//! - [`adjudication`] — Crucible's own judgments, keeping correctness
//!   ([`Verdict`]) deliberately distinct from scope ([`Disposition`]).
//! - [`measure`] — uncertainty and agreement primitives ([`wilson_interval`],
//!   [`proportion`], [`agreement`]) so every reported rate carries an interval.
//! - [`adapter`] — projects Cerberus [`Finding`]s onto Daedalus [`KeyFinding`]
//!   rows ([`findings_from_artifact`], [`to_key_findings`]) so a review and a
//!   key can be compared on equal terms.
//! - [`grade`] — deterministic pre-graders ([`schema_valid`], [`dedup`],
//!   [`key_match`], [`grade`](grade::grade)) that partition a candidate review
//!   against an answer key into matched / disputed / missed before any model or
//!   human judgment. [`recoverable_misses`] re-surfaces the location agreements
//!   the category-strict matcher dropped, so a reported recall is not read as
//!   final.
//!
//! These types are the narrow waist shared by every later step (matcher,
//! confidence interval). They model only the surface the eval consumes;
//! unrecognized fields in real inputs are ignored, not rejected.

mod error;

pub mod adapter;
pub mod adjudication;
pub mod artifact;
pub mod grade;
pub mod key;
pub mod measure;

pub use adapter::{findings_from_artifact, to_key_findings};
pub use adjudication::{Disposition, Verdict};
pub use artifact::{Anchor, AnchorKind, Finding, ReviewArtifact, Severity};
pub use error::{Error, Result};
pub use grade::{
    dedup, grade, key_match, recoverable_misses, schema_valid, GradeResult, Match, LINE_TOLERANCE,
};
pub use key::{AnswerKey, KeyFinding};
pub use measure::{agreement, proportion, wilson_interval};
