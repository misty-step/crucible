//! Crucible's own adjudication primitives.
//!
//! Correctness and scope are deliberately separate axes. Per backlog 002
//! (spike ADJ-2), a finding can be factually true yet ruled out of scope
//! because the change declares no contract covering it. [`Verdict`] judges
//! correctness; [`Disposition`] judges scope. Collapsing them would lose the
//! "correct but out-of-contract" case the eval must represent.

use serde::{Deserialize, Serialize};

/// The correctness verdict for a single finding (snake_case on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Correct and worth keeping.
    Keep,
    /// Correct but trivial — a nit.
    Nit,
    /// Incorrect.
    Wrong,
    /// Not a real finding — noise.
    Noise,
}

/// The scope disposition for a finding, orthogonal to its [`Verdict`].
///
/// A finding can be correct yet fall outside the change's declared contract;
/// `in_scope` records that axis without conflating it with correctness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disposition {
    /// Whether the finding falls within the change's declared contract.
    pub in_scope: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_serializes_snake_case() {
        assert_eq!(serde_json::to_string(&Verdict::Keep).unwrap(), "\"keep\"");
        assert_eq!(serde_json::to_string(&Verdict::Nit).unwrap(), "\"nit\"");
        assert_eq!(serde_json::to_string(&Verdict::Wrong).unwrap(), "\"wrong\"");
        assert_eq!(serde_json::to_string(&Verdict::Noise).unwrap(), "\"noise\"");
    }

    #[test]
    fn verdict_round_trips() {
        for v in [Verdict::Keep, Verdict::Nit, Verdict::Wrong, Verdict::Noise] {
            let s = serde_json::to_string(&v).unwrap();
            let back: Verdict = serde_json::from_str(&s).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn disposition_round_trips_and_is_independent_of_verdict() {
        let d = Disposition { in_scope: false };
        let s = serde_json::to_string(&d).unwrap();
        assert_eq!(s, r#"{"in_scope":false}"#);
        let back: Disposition = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
        assert!(!back.in_scope);
    }
}
