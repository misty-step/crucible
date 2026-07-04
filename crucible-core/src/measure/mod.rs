//! Uncertainty, agreement, and decision primitives for reported rates.
//!
//! Per backlog 003 (measurement rigor), every rate Crucible reports must carry
//! an interval, a model/agentic judge unlocks only above a measured
//! judge-vs-human agreement, and no delta is reported that the data cannot
//! defend. These are the small, pure, deterministic building blocks that
//! machinery rests on. They take counts, slices, and seeds; touch no IO; and
//! never panic on data — degenerate inputs (no trials, empty or misaligned
//! vectors, no discordant pairs, an infeasible effect) yield a defined zero, a
//! `None`, or a refusal rather than a `NaN` or a panic. The one assertion,
//! [`wilson_interval`]'s `debug_assert` that `successes <= n`, guards a *caller
//! contract*, not data: it fires only in debug builds on a caller bug, and the
//! release path clamps instead of panicking.
//!
//! Five concerns, one per submodule, re-exported flat so the public paths stay
//! `crucible_core::measure::*` (and the crate-root re-exports):
//!
//! - [`rate`](self) — a point [`proportion`] and the [`wilson_interval`] around
//!   it.
//! - [`agreement`](self) — raw [`agreement`] and chance-corrected
//!   [`cohen_kappa`] between two judges.
//! - [`paired`](self) — [`PairedComparison`] (McNemar) over two configs' paired
//!   binary outcomes, a matched-pairs rate-delta interval, and the
//!   [`DeltaVerdict`] that refuses a delta inside the noise floor.
//! - [`power`](self) — the [`required_sample_size`] to detect an effect and the
//!   [`power_warning`] that flags an underpowered fixture set.
//! - [`bootstrap`](self) — a deterministic, seeded [`bootstrap_interval`] for a
//!   composite/derived metric, and the seed-robust [`bootstrap_envelope`] whose
//!   directional "excludes 0" decision is invariant to the seed.

mod agreement;
mod bootstrap;
mod normal;
mod paired;
mod power;
mod rate;

pub use agreement::{agreement, cohen_kappa};
pub use bootstrap::{bootstrap_envelope, bootstrap_interval, BootstrapInterval, EnsembleInterval};
pub use paired::{
    paired_rate_delta_interval, DeltaVerdict, PairedComparison, PairedRateDeltaInterval,
};
pub use power::{power_warning, required_sample_size, PowerWarning};
pub use rate::{proportion, wilson_interval};

/// The inverse standard-normal CDF, exposed crate-internally so a caller that
/// needs the `z` quantile for a confidence level — e.g. [`crate::Leaderboard`]
/// turning its `confidence` into the `z` that [`wilson_interval`] takes — reuses
/// the same kernel [`power`] does, rather than hard-coding `1.96`. Not public:
/// callers outside the crate want a finished interval, not a raw quantile.
pub(crate) use normal::inv_normal_cdf;
