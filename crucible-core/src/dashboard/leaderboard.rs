//! The per-`(arena, version)` leaderboard: rank the configs that ran against one
//! frozen answer key, attach a defensible interval to every number, and refuse a
//! ranking the data cannot support.
//!
//! [`super`] ingests the corpus into a [`Dataset`] of [`Eval`] groups; this is
//! the layer that *measures* it. A [`Leaderboard`] is computed from a
//! [`Dataset`] and is a pure render model: the CLI and the HTML dashboard read
//! it, they do not recompute the statistics. Every reported quantity reuses
//! [`crate::measure`] — nothing here re-implements an interval or a test.
//!
//! # Why two different intervals
//!
//! The two headline numbers are different *kinds* of statistic and so get
//! different machinery, which is the whole point of doing this carefully:
//!
//! - [`reward_mean`](LeaderboardEntry::reward_mean) is the mean of a
//!   **continuous** score (`reward ∈ 0..=1`, partial credit like `0.8` is
//!   ordinary). A mean of a continuous variable has no closed-form binomial
//!   interval, so it gets a **percentile bootstrap**
//!   ([`crate::bootstrap_interval`]). Wilson would be *wrong* here — it is an
//!   interval for a proportion, not for the mean of a bounded continuous score.
//! - [`solve_rate`](LeaderboardEntry::solve_rate) is a **binary** proportion: the
//!   fraction of trials that earned a *full* reward (`reward >= 1.0`). A single
//!   proportion is exactly what **Wilson** ([`crate::wilson_interval`]) is for —
//!   small-`n` safe and pinned to `[0, 1]` at the extremes.
//!
//! # The independence unit is the task, not the trial
//!
//! A config runs each task several times; those repeated trials are correlated
//! (same prompt, same key), so they are not independent samples. The bootstrap
//! therefore resamples **whole tasks**, not individual trials — each task is one
//! draw, and all of its trials move together — so the interval reflects
//! *between-task* variance, the variance that actually generalizes. A config
//! evaluated on a single task has no between-task variation to resample and so
//! gets a collapsed (point) interval; read [`n_tasks`](LeaderboardEntry::n_tasks)
//! alongside the bound.
//!
//! `solve_rate`'s Wilson interval is computed at the **task level** for the same
//! reason: a task counts as solved iff its mean reward over trials is `1.0` (every
//! trial earned a full reward), and Wilson is taken over `n = n_tasks`. Computing
//! it over `n_trials` would be pseudoreplication — trials within a task share
//! prompt, key, and model, so they are correlated, not independent draws — and
//! would report a falsely tight interval over a falsely large `n`. The task is the
//! independence unit for both headline numbers.
//!
//! # Reconciliation: the same truth, surfaced — not a new one
//!
//! A config's [`reward_mean`](LeaderboardEntry::reward_mean) point estimate is the
//! **trial-weighted grand mean** over its pooled trials, which is exactly the
//! number Daedalus already records in each run's `summary.json` (`reward_mean`).
//! This is enforced two ways: the bootstrap metric is `Σreward / Σtrials` over the
//! resampled task buckets — *trial-weighted*, so its point equals the grand mean
//! even when tasks have unequal trial counts (they routinely do in the real
//! corpus, where a naive mean-of-task-means drifts from the grand mean by as much
//! as `0.15`) — and a test asserts the computed value matches a fixture's
//! `summary.json` within float tolerance. The leaderboard pools a
//! `composition_hash` across every run in a group, so its reward_mean is the
//! trial-weighted combination of those runs' per-run summary means, never a
//! re-derivation that could disagree with them.
//!
//! # Refusing an indefensible ranking
//!
//! Configs are ranked by `reward_mean` descending, but a higher mean is only a
//! *claim* of superiority until the gap clears the noise floor. Each entry
//! carries a [`vs_next`](LeaderboardEntry::vs_next) comparison against the
//! config ranked immediately below it (so the leader's is the headline
//! "#1 vs #2"), evaluated on the two configs' **shared tasks** by two
//! independent tests:
//!
//! - **McNemar** ([`crate::PairedComparison`]) on the paired per-task binary
//!   solve outcomes — the discordant tasks (one config solved, the other did
//!   not) are the only ones that carry signal, and their imbalance also names a
//!   *direction* (`b > c` favors the higher-ranked config).
//! - A **paired bootstrap** of the per-task reward-mean difference; the interval
//!   that excludes `0` names a direction too (above `0` favors the higher-ranked
//!   config). To make that decision **seed-invariant** it is taken from a
//!   [`bootstrap_envelope`](crate::bootstrap_envelope) — the conservative union
//!   of many seeds — not a single seeded percentile, which can land on either
//!   side of a zero-atom by luck of the seed and flip the verdict.
//!
//! The combined [`verdict`](Pairwise::verdict) is a [`PairwiseVerdict`] with
//! three honest states, never a bare two-sided "signal":
//!
//! - [`Underpowered`](PairwiseVerdict::Underpowered) when the shared-task count
//!   is below [`POWER_FLOOR`] — too little data to run the test at all. This is
//!   *not* the same claim as parity; it is "we cannot tell".
//! - [`Signal`](PairwiseVerdict::Signal) only when at least one test produces a
//!   **directional** result and the tests do not contradict each other. The
//!   verdict carries which config is [`stronger`](Stronger) — and it is named
//!   from the evidence, so a `#1` whose lead lives only on *non-shared* tasks
//!   can correctly come back as "the runner-up is stronger here", never a false
//!   "stronger than runner-up".
//! - [`InsideNoiseFloor`](PairwiseVerdict::InsideNoiseFloor) otherwise: tested on
//!   enough shared tasks, but indistinguishable from noise.
//!
//! ## Why two tests, and the family-wise rate
//!
//! The two tests answer genuinely different questions — McNemar the binary
//! solve-margin, the bootstrap the continuous reward-margin — so a real
//! improvement can show in one and not the other (a partial-credit gap moves the
//! bootstrap while McNemar sees no new full solves). Reporting their OR is by
//! design, not p-hacking. But an OR of two `α`-level tests would inflate the
//! family-wise false-positive rate toward `2α`. So each arm is **Bonferroni-split
//! to `α/2`**: McNemar must clear `p ≤ α/2`, and the bootstrap envelope is taken
//! at the `1 − α/2` level. By the union bound the combined chance of a false
//! directional signal is `≤ α/2 + α/2 = α` — the advertised
//! [`ALPHA`] is restored. The envelope's seed-stability only tightens the
//! bootstrap arm further (it fires only when *every* seed agrees).
//!
//! Everything is deterministic: the bootstrap is seeded, and the seed,
//! resample count, and confidence the board was built with are echoed on the
//! [`Leaderboard`] so a number can be reproduced from the artifact alone — and,
//! because the verdict reads off a seed *envelope*, it reproduces the same
//! *decision* under any seed, not merely the same numbers under one seed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dashboard::{Config, Dataset, Eval};
use crate::measure::inv_normal_cdf;
use crate::{
    bootstrap_envelope, bootstrap_interval, proportion, wilson_interval, DeltaVerdict,
    IntervalMethod,
};
use crate::{Aggregate, PairedComparison};

/// Family-wise significance target for the combined pairwise verdict.
///
/// The conventional `0.05`, and it is the rate that governs the *whole* rank-gap
/// decision, not either test alone: because the verdict ORs two tests, each is
/// Bonferroni-split to [`ALPHA`]`/2` so the union false-positive rate stays at
/// `0.05` (see the module docs). Independent of the interval
/// [`confidence`](Leaderboard::confidence): this is the threshold a *difference*
/// must clear to be called real, not the coverage of a reported interval.
const ALPHA: f64 = 0.05;

/// Confidence level the paired-delta envelope is evaluated at: `1 − α/2`.
///
/// The Bonferroni-corrected level for the bootstrap arm. Fixed here (like
/// [`ALPHA`]) rather than read from the board's display
/// [`confidence`](Leaderboard::confidence), because it sets a *decision*
/// threshold, not the coverage of a displayed marginal interval — the same
/// separation the McNemar `α` already keeps.
const DELTA_CONFIDENCE: f64 = 1.0 - ALPHA / 2.0;

/// Seeds folded into the paired-delta [`bootstrap_envelope`] per comparison.
///
/// The envelope signals a direction only when *all* of these independently
/// seeded intervals exclude `0` on the same side, so the published verdict is
/// invariant to which seed the board was built with. `64` is the floor the
/// design calls for: enough that a genuinely borderline atom is never unanimous
/// (so it refuses, stably) while a clear separation always is (so it signals,
/// stably).
const ENSEMBLE_SEEDS: usize = 64;

/// Fewest shared tasks at which a rank gap is *testable* rather than
/// [`Underpowered`](PairwiseVerdict::Underpowered).
///
/// Below this the comparison is reported `Underpowered` — "too little data to
/// test" — never [`InsideNoiseFloor`](PairwiseVerdict::InsideNoiseFloor), which
/// would falsely claim we tested and found parity. `6` is justified by the
/// discrete arm: McNemar's exact two-sided p-value on `k` all-one-directional
/// discordant pairs is `2·0.5^k`, which only enters the conventional `0.05`
/// significance region at `k = 6` (`2·0.5^5 = 0.0625`, `2·0.5^6 = 0.03125`).
/// Since discordant pairs ≤ shared tasks, a comparison on fewer than six shared
/// tasks cannot even approach a McNemar signal, and a percentile bootstrap over
/// fewer than six task buckets has a tail too coarse to localize off a zero-atom.
/// Below six, neither arm can produce a trustworthy directional exclusion, so we
/// refuse to test. (A clear gap on, say, three tasks is therefore `Underpowered`,
/// not `Signal`: three tasks cannot defend a ranking, however stark the gap — the
/// product thesis, not a regression.)
const POWER_FLOOR: usize = 6;

/// A point estimate with a confidence interval and the method that produced it.
///
/// The interval bounds are inclusive and always bracket [`point`](Self::point)
/// (`lower <= point <= upper`). [`method`](Self::method) names *how* the interval
/// was derived — [`IntervalMethod::Bootstrap`] for a continuous mean,
/// [`IntervalMethod::Wilson`] for a binary proportion — so a renderer can label
/// it without guessing, and [`confidence`](Self::confidence) is the coverage
/// (e.g. `0.95`). A collapsed interval (`lower == upper == point`) is honest, not
/// a bug: it means there was no variation to resample (e.g. a single-task
/// bootstrap), and the reader should weigh the sample size.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    /// The point estimate.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub point: f64,
    /// Lower interval bound at [`confidence`](Self::confidence).
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub lower: f64,
    /// Upper interval bound at [`confidence`](Self::confidence).
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub upper: f64,
    /// Which interval method produced the bounds: `"bootstrap"` or `"wilson"`.
    pub method: IntervalMethod,
    /// The interval's coverage, e.g. `0.95`.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub confidence: f64,
}

impl Estimate {
    /// View as a [`crate::Aggregate`] (`score` + `ci`) for callers that already
    /// render that shape. Drops [`method`](Self::method)/
    /// [`confidence`](Self::confidence), which the [`Aggregate`] does not carry.
    pub fn as_aggregate(&self) -> Aggregate {
        Aggregate {
            score: self.point,
            ci: (self.lower, self.upper),
            paired_delta: None,
        }
    }
}

/// One config's measured standing in a group: identity, both intervals, sample
/// sizes, and the comparison against the next-ranked config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    /// 1-based rank within the group, by [`reward_mean`](Self::reward_mean)
    /// descending (ties broken by `composition_hash` for determinism).
    pub rank: usize,
    /// The config's stable identity — what trials pool on.
    pub composition_hash: String,
    /// Display label (the config's `id`); not identity.
    pub id: String,
    /// Display kind (the config's `kind`); not identity.
    pub kind: String,
    /// Mean continuous reward over pooled trials, with a **bootstrap** interval
    /// resampled over tasks. The point is the trial-weighted grand mean — the
    /// same value the run's `summary.json` records.
    pub reward_mean: Estimate,
    /// Fraction of **tasks** fully solved (every trial earned a full reward), with
    /// a **Wilson** interval over `n_tasks`. Task-level, not trial-level: the task
    /// is the independence unit, so this neither pseudo-replicates correlated
    /// trials nor reports a falsely tight interval.
    pub solve_rate: Estimate,
    /// Pooled trials behind this entry (includes [`n_errors`](Self::n_errors)).
    pub n_trials: usize,
    /// Distinct tasks this config ran — the bootstrap's resample units.
    pub n_tasks: usize,
    /// How many pooled trials were error trials (harness/scorer aborts). They
    /// carry `reward = 0.0`, so they count against both means, never silently
    /// dropped.
    pub n_errors: usize,
    /// Comparison against the config ranked immediately below this one, on their
    /// shared tasks. `None` for the last (or only) entry in the group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vs_next: Option<Pairwise>,
}

/// The McNemar outcome on two configs' paired per-task solve outcomes.
///
/// A render-side mirror of [`crate::PairedComparison`] (which is not itself
/// serializable): the discordant counts, the continuity-corrected χ² statistic,
/// the two-sided p-value, and the noise-floor [`verdict`](Self::verdict) at the
/// Bonferroni-corrected `α/2`. Populated *from* a [`PairedComparison`] so the
/// test math lives in [`crate::measure`], not here. The *direction* this arm
/// favors is not stored as a flag — it is read from the counts: `b > c` favors
/// the higher-ranked config, `c > b` the runner-up.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct McnemarOutcome {
    /// Tasks the higher-ranked config solved and the lower-ranked did not.
    pub b: u64,
    /// Tasks the lower-ranked config solved and the higher-ranked did not.
    pub c: u64,
    /// Continuity-corrected χ² (1 df) statistic; `0` when there are no
    /// discordant tasks.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub statistic: f64,
    /// Two-sided p-value; `1.0` when the configs solved exactly the same tasks.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub p_value: f64,
    /// Whether the paired solve difference clears the noise floor at the
    /// Bonferroni-corrected `α/2` (two-sided). Direction is read from `b`/`c`.
    pub verdict: DeltaVerdict,
}

impl McnemarOutcome {
    /// Build from a computed [`PairedComparison`], attaching the `α/2` verdict
    /// (the Bonferroni share of the family-wise [`ALPHA`]; see module docs).
    fn from_comparison(cmp: PairedComparison) -> Self {
        Self {
            b: cmp.b,
            c: cmp.c,
            statistic: cmp.statistic,
            p_value: cmp.p_value,
            verdict: cmp.verdict(ALPHA / 2.0),
        }
    }
}

/// Which side of `0` the seed-stable paired-delta envelope falls on.
///
/// Read off a [`bootstrap_envelope`](crate::bootstrap_envelope), so it is the
/// *unanimous* conclusion across the seed ensemble, not one seed's percentile:
/// [`Positive`](Self::Positive)/[`Negative`](Self::Negative) mean every member
/// excluded `0` on that side, and [`Zero`](Self::Zero) means at least one did
/// not — the difference is not seed-stably distinguishable from `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaSign {
    /// The whole envelope is above `0`: the higher-ranked config leads on the
    /// shared tasks, stably across seeds.
    Positive,
    /// The whole envelope is below `0`: the lower-ranked config leads.
    Negative,
    /// The envelope includes `0`: indistinguishable from no difference.
    Zero,
}

/// The paired reward-mean difference (higher-ranked − lower-ranked) over shared
/// tasks, with its seed-stable bootstrap envelope and directional sign.
///
/// [`delta`](Self::delta) is the trial-weighted difference on the shared tasks
/// (the only tasks on which a paired comparison is defined) and is seed-free. The
/// bounds are a paired **[`bootstrap_envelope`](crate::bootstrap_envelope)** over
/// those shared tasks — the conservative union of `ENSEMBLE_SEEDS` seeds, not a
/// single percentile — so [`sign`](Self::sign) (taken directly from the bounds)
/// is the same on either side of a zero-atom regardless of seed. The
/// [`confidence`](Self::confidence) is the Bonferroni-corrected
/// `1 − α/2 = 0.975`, wider than the board's marginal intervals on purpose: it is
/// the decision interval that keeps the two-arm family-wise rate at the advertised
/// `α = 0.05`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DeltaEstimate {
    /// Point difference in reward_mean on the shared tasks (higher − lower rank).
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub delta: f64,
    /// Lower bound of the paired bootstrap envelope (the widest member's floor).
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub lower: f64,
    /// Upper bound of the paired bootstrap envelope (the widest member's ceiling).
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub upper: f64,
    /// Always [`IntervalMethod::Bootstrap`]; named for a renderer's benefit.
    pub method: IntervalMethod,
    /// The envelope's coverage: the Bonferroni decision level `1 − α/2 = 0.975`.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub confidence: f64,
    /// Which side of `0` the envelope excludes — the seed-stable directional
    /// conclusion this arm contributes to the combined verdict.
    pub sign: DeltaSign,
}

/// Which of the two compared configs the evidence puts ahead on the shared tasks.
///
/// Carried *inside* a [`PairwiseVerdict::Signal`] so a directional claim can
/// never be rendered for the wrong config. Note the comparison is on **shared
/// tasks** while the ranking is on **all** tasks, so the two can disagree: a
/// config that ranks higher overall (on tasks only it ran) can be the
/// [`RunnerUp`](Self::RunnerUp) here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stronger {
    /// The higher-ranked config (by overall `reward_mean`) is also ahead on the
    /// shared tasks — the rank order is defended.
    Higher,
    /// The lower-ranked config is actually ahead on the shared tasks — the
    /// head-to-head contradicts the overall rank, so the gap must **not** be
    /// read as "the higher-ranked one is better".
    RunnerUp,
}

/// The directional, power-aware verdict on one rank gap.
///
/// Three honest states, serialized internally-tagged on `kind` (so a `Signal`
/// also carries `stronger`, while the other two are bare):
/// `{"kind":"signal","stronger":"higher"}`,
/// `{"kind":"inside_noise_floor"}`, `{"kind":"underpowered"}`.
///
/// ```
/// use crucible_core::{PairwiseVerdict, Stronger};
/// let v = PairwiseVerdict::Signal { stronger: Stronger::Higher };
/// assert_eq!(
///     serde_json::to_string(&v).unwrap(),
///     r#"{"kind":"signal","stronger":"higher"}"#
/// );
/// assert_eq!(
///     serde_json::to_string(&PairwiseVerdict::Underpowered).unwrap(),
///     r#"{"kind":"underpowered"}"#
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PairwiseVerdict {
    /// A defensible, directional win: the named config is ahead on the shared
    /// tasks and a test supports it (seed-stably). Licenses the rank gap *in the
    /// stated direction*.
    Signal {
        /// Which config the evidence puts ahead.
        stronger: Stronger,
    },
    /// Tested on at least `POWER_FLOOR` (6) shared tasks, but the difference is
    /// indistinguishable from noise. The rank gap is not defensible.
    InsideNoiseFloor,
    /// Fewer than `POWER_FLOOR` (6) shared tasks — too little data to run the test.
    /// Distinct from `InsideNoiseFloor`: "we cannot tell", not "they are equal".
    Underpowered,
}

impl PairwiseVerdict {
    /// Whether this is a (directional) signal.
    pub fn is_signal(self) -> bool {
        matches!(self, PairwiseVerdict::Signal { .. })
    }
}

/// A pairwise comparison of two configs in one group, on their shared tasks.
///
/// Carries both independent tests — [`mcnemar`](Self::mcnemar) on paired solve
/// outcomes and [`reward_delta`](Self::reward_delta) on the mean difference — and
/// the combined [`verdict`](Self::verdict): a directional [`PairwiseVerdict`]
/// that is a [`Signal`](PairwiseVerdict::Signal) only when at least one
/// Bonferroni-corrected arm fires with a direction and the arms do not
/// contradict, [`Underpowered`](PairwiseVerdict::Underpowered) below
/// `POWER_FLOOR` (6) shared tasks, and
/// [`InsideNoiseFloor`](PairwiseVerdict::InsideNoiseFloor) otherwise. A directed
/// `Signal` is what licenses ranking one config above the other *in that
/// direction*.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pairwise {
    /// The `composition_hash` of the lower-ranked config being compared against.
    pub against_hash: String,
    /// Tasks both configs ran (the comparison's sample size).
    pub n_shared_tasks: usize,
    /// McNemar's paired test on the per-task binary solve outcomes.
    pub mcnemar: McnemarOutcome,
    /// Seed-stable paired bootstrap envelope of the reward-mean difference.
    pub reward_delta: DeltaEstimate,
    /// Combined directional verdict over the two arms (see [`PairwiseVerdict`]).
    pub verdict: PairwiseVerdict,
}

/// One `(arena_id, arena_version)` group's ranked leaderboard.
///
/// Never pooled across arena versions: the scoring key changes between versions,
/// so a reward from `v1` is not comparable to one from `v2`. Each group is a
/// self-contained ranking over a single fixed key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaderboardGroup {
    /// Arena id, from the trials (never the run directory name).
    pub arena_id: String,
    /// Arena version, from the trials.
    pub arena_version: String,
    /// Distinct tasks any config in the group exercised.
    pub n_tasks: usize,
    /// Configs ranked by [`reward_mean`](LeaderboardEntry::reward_mean)
    /// descending.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<LeaderboardEntry>,
}

/// The whole leaderboard: every `(arena, version)` group, ranked, plus the
/// reproducibility parameters the numbers were computed under.
///
/// Built by [`from_dataset`](Self::from_dataset). The groups are in the
/// [`Dataset`]'s order (`(arena_id, arena_version)` sorted). The echoed
/// [`confidence`](Self::confidence), [`resamples`](Self::resamples), and
/// [`seed`](Self::seed) make the artifact self-describing: the same `Dataset`
/// and the same three parameters reproduce it byte for byte.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Leaderboard {
    /// The ranked groups, never mixed across arena versions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<LeaderboardGroup>,
    /// Interval coverage applied to every estimate, e.g. `0.95`.
    #[serde(serialize_with = "crate::serde_util::serialize_finite")]
    pub confidence: f64,
    /// Bootstrap resamples drawn per interval.
    pub resamples: usize,
    /// Seed for the deterministic bootstrap PRNG.
    pub seed: u64,
}

impl Leaderboard {
    /// Default interval coverage: 95%.
    pub const DEFAULT_CONFIDENCE: f64 = 0.95;
    /// Default bootstrap resamples — enough for stable 95% percentile bounds.
    pub const DEFAULT_RESAMPLES: usize = 10_000;
    /// Default bootstrap seed. Any fixed value makes the board reproducible; this
    /// one is arbitrary.
    pub const DEFAULT_SEED: u64 = 0xC0FF_EE15_5EED_600D;

    /// Compute the leaderboard from a [`Dataset`] with the default confidence
    /// (95%), resamples, and seed.
    ///
    /// ```
    /// use crucible_core::{Dataset, Leaderboard};
    /// // An empty dataset yields an empty board, never a panic.
    /// let board = Leaderboard::from_dataset(&Dataset::default());
    /// assert!(board.groups.is_empty());
    /// assert_eq!(board.confidence, Leaderboard::DEFAULT_CONFIDENCE);
    /// ```
    pub fn from_dataset(dataset: &Dataset) -> Self {
        Self::from_dataset_with(
            dataset,
            Self::DEFAULT_CONFIDENCE,
            Self::DEFAULT_RESAMPLES,
            Self::DEFAULT_SEED,
        )
    }

    /// Compute the leaderboard with explicit `confidence` (`0 < c < 1`),
    /// bootstrap `resamples`, and `seed`.
    ///
    /// Total: a degenerate `confidence` (outside the open unit interval) or zero
    /// `resamples` only collapses the affected intervals onto their point
    /// estimates — the point estimates, ranks, and McNemar verdicts are
    /// unaffected, and nothing panics.
    pub fn from_dataset_with(
        dataset: &Dataset,
        confidence: f64,
        resamples: usize,
        seed: u64,
    ) -> Self {
        let groups = dataset
            .evals
            .iter()
            .map(|eval| group_board(eval, confidence, resamples, seed))
            .collect();
        Leaderboard {
            groups,
            confidence,
            resamples,
            seed,
        }
    }
}

/// Per-task aggregate for one config: the resample bucket the bootstrap draws.
///
/// Holds the task's reward `sum` and trial count `n` (so the trial-weighted mean
/// is `sum / n`) and how many of those trials earned a full reward
/// (`successes`), which drives the binary per-task solve outcome.
#[derive(Clone, Copy)]
struct TaskStat {
    sum: f64,
    n: u64,
    successes: u64,
}

impl TaskStat {
    /// The task's binary solve outcome for the **paired McNemar** test: more than
    /// half its trials earned a full reward. Strict majority, so a single
    /// full-reward trial solves a 1-trial task and a 2-of-3 split solves a 3-trial
    /// one; an even split does not. This is the per-task bit the discordant counts
    /// are built from — a "did this config tend to solve it" collapse.
    fn solved(self) -> bool {
        2 * self.successes > self.n
    }

    /// The task's outcome for the **`solve_rate`** field: solved iff its mean
    /// reward over trials is `1.0` — i.e. *every* trial earned a full reward
    /// (`successes == n`), which (with `reward ∈ [0, 1]`) is exactly a task-mean
    /// of `1.0`. Stricter than [`solved`](Self::solved) on purpose: `solve_rate`
    /// asks "what fraction of tasks did this config *fully* solve", a per-task
    /// (not per-trial) reliability claim.
    fn fully_solved(self) -> bool {
        self.n > 0 && self.successes == self.n
    }
}

/// Reduce a config's pooled trials to one [`TaskStat`] per task id.
fn task_stats(config: &Config) -> BTreeMap<String, TaskStat> {
    let mut stats: BTreeMap<String, TaskStat> = BTreeMap::new();
    for trial in &config.trials {
        let stat = stats.entry(trial.task_id.clone()).or_insert(TaskStat {
            sum: 0.0,
            n: 0,
            successes: 0,
        });
        stat.sum += trial.reward;
        stat.n += 1;
        // A full reward is the binary "solved" bit; partial credit does not count.
        if trial.reward >= 1.0 {
            stat.successes += 1;
        }
    }
    stats
}

/// The `z` quantile for a two-sided interval at `confidence` — what
/// [`wilson_interval`] takes. Falls back to the 95% value if `confidence` is
/// degenerate (so `z` is finite and Wilson stays well-defined).
fn z_for(confidence: f64) -> f64 {
    if confidence <= 0.0 || confidence >= 1.0 {
        return 1.959_963_984_540_054; // Φ⁻¹(0.975)
    }
    inv_normal_cdf(1.0 - (1.0 - confidence) / 2.0)
}

/// Build one group's ranked board.
fn group_board(eval: &Eval, confidence: f64, resamples: usize, seed: u64) -> LeaderboardGroup {
    let z = z_for(confidence);

    let mut entries: Vec<LeaderboardEntry> = eval
        .configs
        .iter()
        .map(|config| entry_for(config, z, confidence, resamples, seed))
        .collect();

    // Rank by reward_mean descending; total_cmp gives a deterministic order even
    // against a stray NaN, and the hash tie-break keeps equal means stable.
    entries.sort_by(|a, b| {
        b.reward_mean
            .point
            .total_cmp(&a.reward_mean.point)
            .then_with(|| a.composition_hash.cmp(&b.composition_hash))
    });

    // Pairwise verdicts need the two configs' trials, so index them by hash and
    // compare each ranked entry to the one below it. Computed into a parallel
    // vec first to avoid borrowing `entries` while mutating it.
    let by_hash: BTreeMap<&str, &Config> = eval
        .configs
        .iter()
        .map(|c| (c.composition_hash.as_str(), c))
        .collect();
    let ordered: Vec<&str> = entries
        .iter()
        .map(|e| e.composition_hash.as_str())
        .collect();
    let mut vs_next: Vec<Option<Pairwise>> = Vec::with_capacity(entries.len());
    for pair in ordered.windows(2) {
        let higher = by_hash.get(pair[0]).copied().expect("ranked hash present");
        let lower = by_hash.get(pair[1]).copied().expect("ranked hash present");
        vs_next.push(Some(pairwise(higher, lower, resamples, seed)));
    }
    vs_next.push(None); // the last entry has no next

    for (rank, (entry, next)) in entries.iter_mut().zip(vs_next).enumerate() {
        entry.rank = rank + 1;
        entry.vs_next = next;
    }

    LeaderboardGroup {
        arena_id: eval.arena_id.clone(),
        arena_version: eval.arena_version.clone(),
        n_tasks: eval.tasks.len(),
        entries,
    }
}

/// Compute one config's entry (ranks/`vs_next` are filled by the caller).
fn entry_for(
    config: &Config,
    z: f64,
    confidence: f64,
    resamples: usize,
    seed: u64,
) -> LeaderboardEntry {
    let stats = task_stats(config);

    // reward_mean: trial-weighted grand mean, bootstrap CI resampled over tasks.
    // The metric is Σreward / Σtrials over the resampled task buckets, so its
    // point equals the grand mean (= summary.json's reward_mean) regardless of
    // how unevenly trials are spread across tasks.
    let buckets: Vec<(f64, u64)> = stats.values().map(|s| (s.sum, s.n)).collect();
    let reward_mean = match bootstrap_interval(&buckets, weighted_mean, resamples, confidence, seed)
    {
        Some(bi) => Estimate {
            point: bi.point,
            lower: bi.lower,
            upper: bi.upper,
            method: IntervalMethod::Bootstrap,
            confidence,
        },
        None => {
            // No tasks or degenerate args: report the point with a collapsed
            // interval rather than nothing.
            let point = config.reward_mean().unwrap_or(0.0);
            Estimate {
                point,
                lower: point,
                upper: point,
                method: IntervalMethod::Bootstrap,
                confidence,
            }
        }
    };

    // solve_rate: TASK-level binary proportion with a Wilson interval. The
    // independence unit is the task, not the trial: a task's repeated trials
    // share prompt/key/model and are correlated, so a trial-level Wilson would
    // pseudo-replicate and report a falsely tight interval over a falsely large
    // n. A task counts as solved iff every one of its trials earned a full reward
    // (task-mean reward 1.0); Wilson is then over n = n_tasks.
    let solved_tasks: u64 = stats.values().filter(|s| s.fully_solved()).count() as u64;
    let n_tasks_u: u64 = stats.len() as u64;
    let (lower, upper) = wilson_interval(solved_tasks, n_tasks_u, z);
    let solve_rate = Estimate {
        point: proportion(solved_tasks, n_tasks_u),
        lower,
        upper,
        method: IntervalMethod::Wilson,
        confidence,
    };

    LeaderboardEntry {
        rank: 0,
        composition_hash: config.composition_hash.clone(),
        id: config.id.clone(),
        kind: config.kind.clone(),
        reward_mean,
        solve_rate,
        n_trials: config.trial_count(),
        n_tasks: stats.len(),
        n_errors: config.error_count(),
        vs_next: None,
    }
}

/// Trial-weighted mean over task buckets: `Σsum / Σn`. `0.0` for an empty
/// resample (degenerate, never from real data) rather than a `NaN`.
fn weighted_mean(buckets: &[(f64, u64)]) -> f64 {
    let (sum, n) = buckets
        .iter()
        .fold((0.0_f64, 0_u64), |(s, c), &(bs, bn)| (s + bs, c + bn));
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

/// Compare two configs (already ordered higher-rank then lower-rank) on their
/// shared tasks, producing the directional, power-aware, seed-stable verdict.
fn pairwise(higher: &Config, lower: &Config, resamples: usize, seed: u64) -> Pairwise {
    let a = task_stats(higher);
    let b = task_stats(lower);

    // McNemar discordant counts + paired delta buckets over shared tasks only.
    let mut mc_b: u64 = 0; // higher solved, lower did not
    let mut mc_c: u64 = 0; // lower solved, higher did not
    let mut delta_buckets: Vec<(f64, u64, f64, u64)> = Vec::new();
    for (task, sa) in &a {
        let Some(sb) = b.get(task) else { continue };
        match (sa.solved(), sb.solved()) {
            (true, false) => mc_b += 1,
            (false, true) => mc_c += 1,
            _ => {}
        }
        delta_buckets.push((sa.sum, sa.n, sb.sum, sb.n));
    }
    let n_shared = delta_buckets.len();

    let mcnemar = McnemarOutcome::from_comparison(PairedComparison::mcnemar(mc_b, mc_c));

    // Paired bootstrap of the trial-weighted reward-mean difference (higher −
    // lower) over the shared tasks, as a SEED-STABLE envelope. Resampling whole
    // (higher, lower) task pairs keeps the comparison paired and the within-task
    // trials together; the envelope (the conservative union of ENSEMBLE_SEEDS
    // seeds, at the Bonferroni DELTA_CONFIDENCE) makes the "excludes 0" decision
    // unanimous across seeds, so the verdict cannot hinge on a seeded percentile
    // landing on either side of a zero-atom.
    let (delta, lo, hi) = match bootstrap_envelope(
        &delta_buckets,
        paired_delta,
        resamples,
        DELTA_CONFIDENCE,
        seed,
        ENSEMBLE_SEEDS,
    ) {
        Some(env) => (env.point, env.lower, env.upper),
        None => {
            // Empty shared set or degenerate args: collapse onto the point.
            let d = paired_delta(&delta_buckets);
            (d, d, d)
        }
    };
    let sign = delta_sign(lo, hi);
    let verdict = combine_verdict(n_shared, &mcnemar, sign);

    Pairwise {
        against_hash: lower.composition_hash.clone(),
        n_shared_tasks: n_shared,
        mcnemar,
        reward_delta: DeltaEstimate {
            delta,
            lower: lo,
            upper: hi,
            method: IntervalMethod::Bootstrap,
            confidence: DELTA_CONFIDENCE,
            sign,
        },
        verdict,
    }
}

/// The seed-stable directional read of a paired-delta envelope: which side of `0`
/// the *whole* envelope clears, or [`DeltaSign::Zero`] if it straddles `0`.
fn delta_sign(lower: f64, upper: f64) -> DeltaSign {
    if lower > 0.0 {
        DeltaSign::Positive
    } else if upper < 0.0 {
        DeltaSign::Negative
    } else {
        DeltaSign::Zero
    }
}

/// Combine the two Bonferroni-corrected arms into the directional, power-aware
/// verdict.
///
/// Power floor first: below [`POWER_FLOOR`] shared tasks the gap is
/// [`Underpowered`](PairwiseVerdict::Underpowered) — never a signal, and never the
/// distinct claim of parity. Above it, each arm yields a *direction* or nothing:
/// McNemar from its discordant imbalance (only once it cleared `α/2`), the
/// bootstrap from its seed-stable [`DeltaSign`]. A [`Signal`](PairwiseVerdict::Signal)
/// needs at least one arm to name a direction and the arms not to contradict —
/// two arms pointing opposite ways is evidence *against* a clean ranking, so it is
/// refused as [`InsideNoiseFloor`](PairwiseVerdict::InsideNoiseFloor).
fn combine_verdict(n_shared: usize, mcnemar: &McnemarOutcome, sign: DeltaSign) -> PairwiseVerdict {
    if n_shared < POWER_FLOOR {
        return PairwiseVerdict::Underpowered;
    }
    // McNemar names a direction only when it cleared its α/2 threshold.
    let mcnemar_dir = if mcnemar.verdict.is_signal() {
        if mcnemar.b > mcnemar.c {
            Some(Stronger::Higher)
        } else if mcnemar.c > mcnemar.b {
            Some(Stronger::RunnerUp)
        } else {
            None // |b - c| == 0 cannot be significant; explicit for totality
        }
    } else {
        None
    };
    let bootstrap_dir = match sign {
        DeltaSign::Positive => Some(Stronger::Higher),
        DeltaSign::Negative => Some(Stronger::RunnerUp),
        DeltaSign::Zero => None,
    };
    match (mcnemar_dir, bootstrap_dir) {
        // Both arms agree, or exactly one fires: a directional signal.
        (Some(a), Some(b)) if a == b => PairwiseVerdict::Signal { stronger: a },
        (Some(a), None) | (None, Some(a)) => PairwiseVerdict::Signal { stronger: a },
        // Arms contradict: not a clean ranking — refuse.
        (Some(_), Some(_)) => PairwiseVerdict::InsideNoiseFloor,
        // Neither fires: tested, indistinguishable.
        (None, None) => PairwiseVerdict::InsideNoiseFloor,
    }
}

/// Trial-weighted reward-mean difference (higher − lower) over shared-task
/// buckets `(sum_higher, n_higher, sum_lower, n_lower)`.
fn paired_delta(buckets: &[(f64, u64, f64, u64)]) -> f64 {
    let (sa, na, sb, nb) = buckets.iter().fold(
        (0.0_f64, 0_u64, 0.0_f64, 0_u64),
        |(sa, na, sb, nb), &(ws, wn, ls, ln)| (sa + ws, na + wn, sb + ls, nb + ln),
    );
    let higher = if na == 0 { 0.0 } else { sa / na as f64 };
    let lower = if nb == 0 { 0.0 } else { sb / nb as f64 };
    higher - lower
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Dataset;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A self-deleting scratch tree, so a synthetic arenas/runs fixture runs
    /// through the real [`Dataset::load`] path with no committed fixtures.
    struct TempTree {
        root: PathBuf,
    }

    impl TempTree {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "crucible-leaderboard-test-{tag}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&root).expect("create scratch root");
            Self { root }
        }

        fn write(&self, rel: &str, contents: &str) {
            let path = self.root.join(rel);
            std::fs::create_dir_all(path.parent().expect("rel has a parent"))
                .expect("create parent dirs");
            std::fs::write(path, contents).expect("write fixture file");
        }

        fn arenas(&self) -> PathBuf {
            self.root.join("arenas")
        }

        fn runs(&self) -> PathBuf {
            self.root.join("runs")
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// Float comparison tolerance for interval bounds and points.
    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// One `trials.jsonl` line. `reward < 0.0` flags an error trial (the harness
    /// aborts before scoring, so `expected_defects` is null and `error` is set);
    /// the recorded reward is then `0.0`, as in the real corpus.
    fn trial(
        arena: &str,
        version: &str,
        hash: &str,
        id: &str,
        task: &str,
        idx: i64,
        reward: f64,
    ) -> String {
        let is_error = reward < 0.0;
        let reward = if is_error { 0.0 } else { reward };
        let (error, expected) = if is_error {
            (r#""harness abort""#, "null")
        } else {
            ("null", "1")
        };
        let solved = if reward >= 1.0 { 1 } else { 0 };
        format!(
            r#"{{"run_id":"r","arena_id":"{arena}","arena_version":"{version}","task_id":"{task}","trial":{idx},"candidate_id":"{id}","candidate_kind":"k","composition_hash":"{hash}","model":null,"cost_usd":null,"error":{error},"wall_ms":10,"reward":{reward},"recall":{reward},"matched":[],"false_positives":0,"expected_defects":{expected},"scorer_error":null,"daedalus_solved":{solved}}}"#
        )
    }

    /// A small board built from one run of `lines`, default parameters but a
    /// small resample count for test speed.
    fn board_from(tag: &str, lines: &[String]) -> Leaderboard {
        let tree = TempTree::new(tag);
        tree.write("runs/r/trials.jsonl", &format!("{}\n", lines.join("\n")));
        let ds = Dataset::load(tree.arenas(), tree.runs());
        Leaderboard::from_dataset_with(&ds, 0.95, 2000, Leaderboard::DEFAULT_SEED)
    }

    // ----- Reconciliation: the leaderboard surfaces summary.json's number -----

    #[test]
    fn reward_mean_reconciles_with_summary_json_under_nonuniform_trials() {
        // The hard case the real corpus actually contains: trials are spread
        // UNEVENLY across tasks (here 1 vs 5). The trial-weighted grand mean is
        // (0.0 + 5·1.0) / 6 = 0.8333…; a naive mean-of-task-means would be
        // (0.0 + 1.0) / 2 = 0.5. Daedalus records the grand mean in
        // summary.json, so the leaderboard must too — proving we surface the
        // same truth, not a new one, even when trial counts differ per task.
        let tree = TempTree::new("reconcile");
        let mut lines = vec![trial("arena-x", "0.1.0", "h", "cfg", "t-rare", 1, 0.0)];
        for i in 0..5 {
            lines.push(trial("arena-x", "0.1.0", "h", "cfg", "t-common", i, 1.0));
        }
        tree.write("runs/r/trials.jsonl", &format!("{}\n", lines.join("\n")));
        // The summary.json Daedalus would write alongside it (reward_mean is the
        // 4-dp-rounded grand mean, exactly as the real files store it).
        tree.write(
            "runs/r/summary.json",
            r#"{"cfg":{"composition_hash":"h","kind":"k","tasks":{},"trials":6,"errors":0,"cost_usd_total":0.0,"cost_known":true,"reward_mean":0.8333}}"#,
        );

        let ds = Dataset::load(tree.arenas(), tree.runs());
        let board = Leaderboard::from_dataset_with(&ds, 0.95, 2000, Leaderboard::DEFAULT_SEED);
        let entry = &board.groups[0].entries[0];

        // Exact correctness against the hand-computed grand mean.
        assert!(
            close(entry.reward_mean.point, 5.0 / 6.0, 1e-9),
            "computed reward_mean {} is not the trial-weighted grand mean",
            entry.reward_mean.point
        );
        // And it is NOT the mean-of-task-means (0.5) a naive design would emit.
        assert!(
            !close(entry.reward_mean.point, 0.5, 1e-3),
            "reward_mean collapsed to mean-of-task-means — trial weighting lost"
        );

        // Reconciliation against the value parsed from summary.json itself.
        let summ: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tree.runs().join("r/summary.json")).unwrap(),
        )
        .unwrap();
        let summary_mean = summ["cfg"]["reward_mean"].as_f64().unwrap();
        assert!(
            close(entry.reward_mean.point, summary_mean, 1.5e-3),
            "reward_mean {} disagrees with summary.json {summary_mean}",
            entry.reward_mean.point
        );
        // The point is always bracketed by its own interval.
        assert!(entry.reward_mean.lower <= entry.reward_mean.point);
        assert!(entry.reward_mean.point <= entry.reward_mean.upper);
    }

    // ----- The two intervals carry the right method and bracket their point ---

    #[test]
    fn reward_uses_bootstrap_and_solve_uses_wilson_with_known_values() {
        // 10 tasks, 1 trial each: 8 fully solved (reward 1.0), 2 partial (0.8,
        // never a full solve). solve_rate is the TASK-level 8/10 — the fraction of
        // tasks fully solved, n = n_tasks — and reward_mean is the continuous
        // (8·1.0 + 2·0.8)/10 = 0.96.
        let mut lines = Vec::new();
        for i in 0..8 {
            lines.push(trial("a", "1", "h", "c", &format!("t{i}"), 1, 1.0));
        }
        lines.push(trial("a", "1", "h", "c", "t8", 1, 0.8));
        lines.push(trial("a", "1", "h", "c", "t9", 1, 0.8));

        let board = board_from("methods", &lines);
        let e = &board.groups[0].entries[0];

        assert_eq!(e.reward_mean.method, IntervalMethod::Bootstrap);
        assert_eq!(e.solve_rate.method, IntervalMethod::Wilson);
        assert!(
            close(e.reward_mean.point, 0.96, 1e-9),
            "{}",
            e.reward_mean.point
        );
        // 8 of 10 tasks fully solved — the task is the independence unit.
        assert!(
            close(e.solve_rate.point, 0.8, 1e-9),
            "{}",
            e.solve_rate.point
        );
        // Wilson 95% for 8/10 is the textbook ~[0.49, 0.94] — and now n is tasks.
        assert!(
            close(e.solve_rate.lower, 0.49, 0.02),
            "wilson lo {}",
            e.solve_rate.lower
        );
        assert!(
            close(e.solve_rate.upper, 0.94, 0.02),
            "wilson hi {}",
            e.solve_rate.upper
        );
        assert_eq!(e.n_trials, 10);
        assert_eq!(e.n_tasks, 10);
        assert_eq!(e.n_errors, 0);
        // Both intervals bracket their point.
        for est in [e.reward_mean, e.solve_rate] {
            assert!(est.lower <= est.point && est.point <= est.upper);
            assert_eq!(est.confidence, 0.95);
        }
    }

    // ----- Ranking and the noise-floor verdict the rank gap must clear --------

    #[test]
    fn configs_rank_by_reward_mean_and_a_clear_gap_is_signal() {
        // `strong` solves all 12 shared tasks; `weak` solves none of them. Both
        // McNemar (b=12, c=0) and the constant +1.0 delta clear the floor.
        let mut lines = Vec::new();
        for i in 0..12 {
            let task = format!("t{i}");
            lines.push(trial("a", "1", "strong", "strong", &task, 1, 1.0));
            lines.push(trial("a", "1", "weak", "weak", &task, 1, 0.0));
        }
        let board = board_from("rank", &lines);
        let g = &board.groups[0];

        assert_eq!(g.entries.len(), 2);
        assert_eq!(
            g.entries[0].composition_hash, "strong",
            "higher mean ranks #1"
        );
        assert_eq!(g.entries[0].rank, 1);
        assert_eq!(g.entries[1].composition_hash, "weak");
        assert_eq!(g.entries[1].rank, 2);

        let vs = g.entries[0]
            .vs_next
            .as_ref()
            .expect("leader compares to runner-up");
        assert_eq!(vs.against_hash, "weak");
        assert_eq!(vs.n_shared_tasks, 12);
        assert_eq!(vs.mcnemar.b, 12);
        assert_eq!(vs.mcnemar.c, 0);
        assert!(
            vs.mcnemar.p_value < 0.05,
            "mcnemar p {}",
            vs.mcnemar.p_value
        );
        assert_eq!(vs.mcnemar.verdict, DeltaVerdict::Signal);
        assert!(
            vs.reward_delta.delta > 0.9,
            "delta {}",
            vs.reward_delta.delta
        );
        // The +1.0 delta is constant across tasks → the envelope excludes 0 on the
        // higher side for every seed.
        assert_eq!(vs.reward_delta.sign, DeltaSign::Positive);
        // Directional: the higher-ranked config really is the stronger one.
        assert_eq!(
            vs.verdict,
            PairwiseVerdict::Signal {
                stronger: Stronger::Higher
            }
        );
        // The runner-up has nothing below it.
        assert!(g.entries[1].vs_next.is_none());
    }

    #[test]
    fn below_the_power_floor_is_underpowered_even_for_a_stark_gap() {
        // Power-honesty: below POWER_FLOOR (6) shared tasks the verdict is
        // Underpowered — "too little data to test" — never InsideNoiseFloor (a
        // claim of parity) and never Signal. This holds even for a *perfect
        // shutout*: `hi` solves all 5 shared tasks, `lo` solves none. McNemar
        // b=5,c=0 gives p = 2·0.5^5 = 0.0625 (not significant), and five tasks
        // cannot defend a ranking however stark the gap looks.
        let mut lines = Vec::new();
        for i in 0..5 {
            let task = format!("t{i}");
            lines.push(trial("a", "1", "hi", "hi", &task, 1, 1.0));
            lines.push(trial("a", "1", "lo", "lo", &task, 1, 0.0));
        }
        let board = board_from("underpowered", &lines);
        let vs = board.groups[0].entries[0]
            .vs_next
            .as_ref()
            .expect("two configs compared");
        assert_eq!(
            vs.n_shared_tasks, 5,
            "five shared tasks, below the floor of 6"
        );
        assert_eq!(
            vs.verdict,
            PairwiseVerdict::Underpowered,
            "a stark gap on five tasks is still too little data to test"
        );
        assert!(!vs.verdict.is_signal());

        // A tiny gap on few tasks is the same refusal — Underpowered, not noise.
        let tiny = [
            trial("a", "1", "hi", "hi", "t1", 1, 1.0),
            trial("a", "1", "hi", "hi", "t2", 1, 1.0),
            trial("a", "1", "lo", "lo", "t1", 1, 1.0),
            trial("a", "1", "lo", "lo", "t2", 1, 0.0),
        ];
        let board = board_from("tiny", &tiny);
        let vs = board.groups[0].entries[0].vs_next.as_ref().unwrap();
        assert_eq!(vs.n_shared_tasks, 2);
        assert_eq!(vs.verdict, PairwiseVerdict::Underpowered);
    }

    #[test]
    fn continuous_gap_with_no_full_solves_is_signal_via_the_bootstrap_alone() {
        // Every task: higher scores 0.9, lower scores 0.1 — a real, consistent
        // reward gap, but NEITHER ever earns a full reward, so McNemar sees zero
        // discordant solves and refuses. The paired bootstrap of the +0.8 delta
        // carries the verdict. This is why both tests exist: McNemar alone is
        // blind to a partial-credit gap.
        let mut lines = Vec::new();
        for i in 0..8 {
            let task = format!("t{i}");
            lines.push(trial("a", "1", "hi", "hi", &task, 1, 0.9));
            lines.push(trial("a", "1", "lo", "lo", &task, 1, 0.1));
        }
        let board = board_from("partial", &lines);
        let vs = board.groups[0].entries[0].vs_next.as_ref().unwrap();

        assert_eq!(vs.mcnemar.b, 0, "no full solves to be discordant on");
        assert_eq!(vs.mcnemar.c, 0);
        assert_eq!(
            vs.mcnemar.verdict,
            DeltaVerdict::InsideNoiseFloor,
            "McNemar is blind to the partial-credit gap"
        );
        assert!(
            close(vs.reward_delta.delta, 0.8, 1e-9),
            "delta {}",
            vs.reward_delta.delta
        );
        assert!(vs.reward_delta.lower > 0.0, "delta envelope must exclude 0");
        assert_eq!(vs.reward_delta.sign, DeltaSign::Positive);
        // The bootstrap arm alone carries a directional signal for the higher config.
        assert_eq!(
            vs.verdict,
            PairwiseVerdict::Signal {
                stronger: Stronger::Higher
            },
            "the bootstrap carries it"
        );
    }

    // ----- Seed-invariance: the moat must not depend on a seed ---------------

    #[test]
    fn the_verdict_is_identical_across_distinct_seeds() {
        // The seed-flip case by construction: `hi` fully solves 3 of 8 shared
        // tasks and scores 0.0 on the other 5; `lo` solves none. delta = 0.375,
        // McNemar b=3/c=0 (p = 0.25). The paired-bootstrap distribution piles
        // ~2.3% of its mass exactly on a 0 atom — (5/8)^8 ≈ 0.023 of resamples
        // draw none of the three solved tasks — so a single seed's lower
        // percentile lands on either side of that atom by luck. Measured on the
        // pre-fix single-seed code, this exact shape shipped Signal under 136/200
        // seeds and InsideNoiseFloor under the other 64 — the verdict was a
        // coin-flip. The seed-stable envelope must publish ONE verdict for all.
        let tree = TempTree::new("seedstable");
        let mut lines = Vec::new();
        for i in 0..3 {
            lines.push(trial("a", "1", "hi", "hi", &format!("t{i}"), 1, 1.0));
        }
        for i in 3..8 {
            lines.push(trial("a", "1", "hi", "hi", &format!("t{i}"), 1, 0.0));
        }
        for i in 0..8 {
            lines.push(trial("a", "1", "lo", "lo", &format!("t{i}"), 1, 0.0));
        }
        tree.write("runs/r/trials.jsonl", &format!("{}\n", lines.join("\n")));
        let ds = Dataset::load(tree.arenas(), tree.runs());

        let verdicts: Vec<PairwiseVerdict> =
            [1u64, 2, 7, 42, 99, 1000, 31337, 0xC0FFEE, 0xDEAD_BEEF]
                .into_iter()
                .map(|seed| {
                    let board = Leaderboard::from_dataset_with(&ds, 0.95, 2000, seed);
                    board.groups[0].entries[0]
                        .vs_next
                        .as_ref()
                        .expect("hi vs lo")
                        .verdict
                })
                .collect();

        assert!(
            verdicts.windows(2).all(|w| w[0] == w[1]),
            "verdict flips across seeds — the moat depends on a seed: {verdicts:?}"
        );
        // The honest verdict for this borderline gap is a refusal: McNemar
        // p = 0.25 ≫ α/2, and the zero-atom keeps the envelope from excluding 0.
        assert_eq!(
            verdicts[0],
            PairwiseVerdict::InsideNoiseFloor,
            "borderline gap should refuse, stably: {verdicts:?}"
        );
    }

    // ----- Directional: a signal never claims the wrong winner ---------------

    #[test]
    fn a_signal_names_the_runner_up_when_the_overall_rank_is_contradicted() {
        // Ranking is by OVERALL reward_mean; the comparison is on SHARED tasks, so
        // they can disagree. `toprank` wins overall on tasks only it ran, but on
        // the 8 shared tasks the runner-up dominates head-to-head. A two-sided
        // signal must therefore name the RUNNER-UP as stronger — never a false
        // "higher is stronger" for the #1-ranked config (the direction-agnostic
        // bug).
        let mut lines = Vec::new();
        // 8 shared tasks: toprank 0.0, rival 1.0.
        for i in 0..8 {
            let task = format!("s{i}");
            lines.push(trial("a", "1", "toprank", "toprank", &task, 1, 0.0));
            lines.push(trial("a", "1", "rival", "rival", &task, 1, 1.0));
        }
        // toprank's solo solves lift its overall mean to 4/12 ≈ 0.333.
        for i in 0..4 {
            lines.push(trial(
                "a",
                "1",
                "toprank",
                "toprank",
                &format!("x{i}"),
                1,
                1.0,
            ));
        }
        // rival's solo misses drag its overall to 8/28 ≈ 0.286 (below toprank).
        for i in 0..20 {
            lines.push(trial("a", "1", "rival", "rival", &format!("y{i}"), 1, 0.0));
        }
        let board = board_from("contradicted", &lines);
        let g = &board.groups[0];
        assert_eq!(
            g.entries[0].composition_hash, "toprank",
            "toprank ranks #1 overall"
        );
        assert_eq!(g.entries[1].composition_hash, "rival");

        let vs = g.entries[0].vs_next.as_ref().expect("toprank vs rival");
        assert_eq!(vs.n_shared_tasks, 8);
        // On shared tasks the delta is a constant −1.0 → envelope below 0.
        assert!(
            vs.reward_delta.delta < 0.0,
            "delta {}",
            vs.reward_delta.delta
        );
        assert_eq!(vs.reward_delta.sign, DeltaSign::Negative);
        // McNemar: c=8 (rival solved, toprank did not), b=0 → favors the runner-up.
        assert_eq!(vs.mcnemar.b, 0);
        assert_eq!(vs.mcnemar.c, 8);
        assert_eq!(
            vs.verdict,
            PairwiseVerdict::Signal {
                stronger: Stronger::RunnerUp
            },
            "a two-sided signal must never be rendered as the higher config winning"
        );
    }

    // ----- solve_rate is task-level (no pseudoreplication) -------------------

    #[test]
    fn solve_rate_is_task_level_and_reconciles_against_raw() {
        // Three tasks, uneven trials. A task is solved iff EVERY trial earned a
        // full reward (task-mean 1.0):
        //   t1: [1.0, 1.0, 1.0] → solved
        //   t2: [1.0, 1.0, 0.0] → NOT solved (one miss)
        //   t3: [1.0, 1.0]      → solved
        // Task-level solve_rate = 2/3 over n = 3 tasks. A trial-level count would
        // be 6/8 = 0.75 over a falsely large n = 8 — the pseudoreplication this
        // fixes. reward_mean stays the trial-weighted grand mean 7/8.
        let lines = [
            trial("a", "1", "h", "c", "t1", 1, 1.0),
            trial("a", "1", "h", "c", "t1", 2, 1.0),
            trial("a", "1", "h", "c", "t1", 3, 1.0),
            trial("a", "1", "h", "c", "t2", 1, 1.0),
            trial("a", "1", "h", "c", "t2", 2, 1.0),
            trial("a", "1", "h", "c", "t2", 3, 0.0),
            trial("a", "1", "h", "c", "t3", 1, 1.0),
            trial("a", "1", "h", "c", "t3", 2, 1.0),
        ];
        let board = board_from("tasklevel", &lines);
        let e = &board.groups[0].entries[0];

        assert_eq!(e.n_tasks, 3, "the independence unit is the task");
        assert_eq!(e.n_trials, 8);
        assert!(
            close(e.solve_rate.point, 2.0 / 3.0, 1e-9),
            "task-level solve_rate {} != 2/3",
            e.solve_rate.point
        );
        // NOT the trial-level 6/8 a pseudoreplicating design would emit.
        assert!(
            !close(e.solve_rate.point, 0.75, 1e-3),
            "solve_rate collapsed to the trial level"
        );
        // reward_mean point is untouched: trial-weighted grand mean 7/8.
        assert!(
            close(e.reward_mean.point, 7.0 / 8.0, 1e-9),
            "reward_mean {} != 7/8",
            e.reward_mean.point
        );
        // Wilson is over n = 3 tasks and still brackets the point.
        assert!(e.solve_rate.lower <= e.solve_rate.point);
        assert!(e.solve_rate.point <= e.solve_rate.upper);
    }

    // ----- Cross-version groups are never pooled -----------------------------

    #[test]
    fn arena_versions_are_separate_groups_never_pooled() {
        // Same arena id, two versions; the SAME composition_hash ran in both.
        // The board must keep them in two groups (the scoring key differs), each
        // with its own single entry — never one pooled mean.
        let lines = [
            trial("pr-review", "0.1.0", "h", "c", "t1", 1, 0.2),
            trial("pr-review", "0.2.0", "h", "c", "t1", 1, 1.0),
        ];
        let board = board_from("versions", &lines);
        assert_eq!(board.groups.len(), 2, "two arena versions, two groups");
        let g1 = board
            .groups
            .iter()
            .find(|g| g.arena_version == "0.1.0")
            .unwrap();
        let g2 = board
            .groups
            .iter()
            .find(|g| g.arena_version == "0.2.0")
            .unwrap();
        assert!(close(g1.entries[0].reward_mean.point, 0.2, 1e-9));
        assert!(close(g2.entries[0].reward_mean.point, 1.0, 1e-9));
        assert_eq!(g1.arena_id, "pr-review");
        assert_eq!(g2.arena_id, "pr-review");
    }

    // ----- Totality and degenerate inputs ------------------------------------

    #[test]
    fn empty_dataset_yields_an_empty_board() {
        let board = Leaderboard::from_dataset(&Dataset::default());
        assert!(board.groups.is_empty());
        assert_eq!(board.confidence, Leaderboard::DEFAULT_CONFIDENCE);
        assert_eq!(board.resamples, Leaderboard::DEFAULT_RESAMPLES);
        assert_eq!(board.seed, Leaderboard::DEFAULT_SEED);
    }

    #[test]
    fn single_task_config_gets_a_collapsed_reward_interval() {
        // One task → no between-task variation to resample → the bootstrap
        // interval collapses onto the point. Honest, not a bug; the reader sees
        // n_tasks == 1.
        let lines = [
            trial("a", "1", "h", "c", "solo", 1, 1.0),
            trial("a", "1", "h", "c", "solo", 2, 0.0),
        ];
        let board = board_from("solo", &lines);
        let e = &board.groups[0].entries[0];
        assert_eq!(e.n_tasks, 1);
        assert!(close(e.reward_mean.point, 0.5, 1e-9));
        assert!(
            close(e.reward_mean.lower, e.reward_mean.upper, 1e-12),
            "single-task reward interval should collapse to a point"
        );
    }

    #[test]
    fn error_trials_count_as_zero_reward_and_non_solves() {
        // 1 full-reward trial + 1 error trial (reward 0.0). reward_mean = 0.5,
        // solve_rate = 1/2, and the error is surfaced in n_errors — never
        // silently dropped.
        let lines = [
            trial("a", "1", "h", "c", "t1", 1, 1.0),
            trial("a", "1", "h", "c", "t2", 1, -1.0), // error trial
        ];
        let board = board_from("errors", &lines);
        let e = &board.groups[0].entries[0];
        assert_eq!(e.n_trials, 2);
        assert_eq!(e.n_errors, 1);
        assert!(close(e.reward_mean.point, 0.5, 1e-9));
        assert!(close(e.solve_rate.point, 0.5, 1e-9));
    }

    #[test]
    fn degenerate_confidence_collapses_intervals_without_panicking() {
        // confidence outside (0,1): point estimates and ranks survive, intervals
        // just collapse onto their points. Totality, not a panic.
        let lines = [
            trial("a", "1", "h", "c", "t1", 1, 1.0),
            trial("a", "1", "h", "c", "t2", 1, 0.0),
        ];
        let tree = TempTree::new("degen");
        tree.write("runs/r/trials.jsonl", &format!("{}\n", lines.join("\n")));
        let ds = Dataset::load(tree.arenas(), tree.runs());
        let board = Leaderboard::from_dataset_with(&ds, 1.5, 0, 1);
        let e = &board.groups[0].entries[0];
        assert!(close(e.reward_mean.point, 0.5, 1e-9));
        // Wilson z falls back to the 95% value, so the solve interval is still a
        // real, finite interval rather than a NaN.
        assert!(e.solve_rate.lower.is_finite() && e.solve_rate.upper.is_finite());
    }

    // ----- The artifact is serde-stable with named methods -------------------

    #[test]
    fn leaderboard_round_trips_and_names_its_interval_methods() {
        let lines = [
            trial("a", "1", "hi", "hi", "t1", 1, 1.0),
            trial("a", "1", "hi", "hi", "t2", 1, 1.0),
            trial("a", "1", "lo", "lo", "t1", 1, 0.0),
            trial("a", "1", "lo", "lo", "t2", 1, 0.0),
        ];
        let board = board_from("serde", &lines);
        let json = serde_json::to_string(&board).expect("serialize");
        // The methods are named on the wire so a renderer never guesses.
        assert!(
            json.contains("\"bootstrap\""),
            "missing bootstrap method: {json}"
        );
        assert!(json.contains("\"wilson\""), "missing wilson method: {json}");
        let back: Leaderboard = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(board, back, "leaderboard must round-trip byte-for-byte");
    }

    // ----- Real corpus: seed-invariant verdicts, same truth as Daedalus ------

    /// Against a local Daedalus checkout (`CRUCIBLE_DAEDALUS_DIR`): every
    /// published `vs_next` verdict must be identical across many bootstrap seeds.
    /// This is the moat's headline guarantee on real data — it covers the
    /// `pr-review-correctness-v0` `0.3.0` cells (oracle/oneshot/null) whose
    /// single-seed verdict used to flip 11–13% of the time. Ignored by default so
    /// the gate never depends on the checkout; run with
    /// `cargo test -p crucible-core -- --ignored` and the env var set.
    #[test]
    #[ignore = "requires a local Daedalus checkout via CRUCIBLE_DAEDALUS_DIR"]
    fn real_corpus_verdicts_are_seed_invariant() {
        let Ok(root) = std::env::var("CRUCIBLE_DAEDALUS_DIR") else {
            return;
        };
        let root = PathBuf::from(root);
        let ds = Dataset::load(root.join("arenas"), root.join("runs"));
        // ≥8 distinct seeds, including the board default.
        let seeds = [
            Leaderboard::DEFAULT_SEED,
            1,
            2,
            7,
            42,
            99,
            1000,
            31337,
            0xDEAD_BEEF,
        ];
        let boards: Vec<Leaderboard> = seeds
            .iter()
            .map(|&s| Leaderboard::from_dataset_with(&ds, 0.95, 4000, s))
            .collect();

        let mut comparisons = 0usize;
        for (gi, group) in boards[0].groups.iter().enumerate() {
            for ei in 0..group.entries.len() {
                let verdicts: Vec<Option<PairwiseVerdict>> = boards
                    .iter()
                    .map(|b| b.groups[gi].entries[ei].vs_next.as_ref().map(|p| p.verdict))
                    .collect();
                assert!(
                    verdicts.windows(2).all(|w| w[0] == w[1]),
                    "{} {} entry {ei}: verdict flips across seeds: {verdicts:?}",
                    group.arena_id,
                    group.arena_version
                );
                if verdicts[0].is_some() {
                    comparisons += 1;
                }
            }
        }
        assert!(
            comparisons > 0,
            "expected at least one real pairwise comparison to check"
        );
    }

    // ----- Real corpus: same truth as Daedalus, at scale ---------------------

    /// Against a local Daedalus checkout (`CRUCIBLE_DAEDALUS_DIR`): every config's
    /// leaderboard `reward_mean` must equal the `reward_mean` Daedalus recorded in
    /// the contributing run's `summary.json`, and every interval must bracket its
    /// point with ranks in order. Ignored by default so the gate never depends on
    /// the checkout; run with `cargo test -p crucible-core -- --ignored` and the
    /// env var set.
    #[test]
    #[ignore = "requires a local Daedalus checkout via CRUCIBLE_DAEDALUS_DIR"]
    fn real_corpus_reconciles_and_is_well_formed() {
        let Ok(root) = std::env::var("CRUCIBLE_DAEDALUS_DIR") else {
            return;
        };
        let root = PathBuf::from(root);
        let ds = Dataset::load(root.join("arenas"), root.join("runs"));
        let board = Leaderboard::from_dataset(&ds);

        // Structural integrity on real data: ranks dense and ascending, every
        // point bracketed by its interval, every solve_rate a real proportion.
        for g in &board.groups {
            for (i, e) in g.entries.iter().enumerate() {
                assert_eq!(e.rank, i + 1, "ranks must be 1..=n in order");
                if i + 1 < g.entries.len() {
                    assert!(
                        e.reward_mean.point + 1e-12 >= g.entries[i + 1].reward_mean.point,
                        "entries not sorted by reward_mean desc"
                    );
                }
                for est in [e.reward_mean, e.solve_rate] {
                    assert!(
                        est.lower <= est.point + 1e-9 && est.point <= est.upper + 1e-9,
                        "point {} escaped interval [{}, {}]",
                        est.point,
                        est.lower,
                        est.upper
                    );
                }
                assert!((0.0..=1.0).contains(&e.solve_rate.point));
            }
        }

        // Reconciliation against summary.json, per run, using the crate's own
        // Trial parsing: the grand mean we compute per composition_hash must match
        // Daedalus's recorded reward_mean within its 4-dp rounding.
        let mut checked = 0_usize;
        for entry in std::fs::read_dir(root.join("runs"))
            .expect("runs dir")
            .flatten()
        {
            let dir = entry.path();
            let Ok(summary_text) = std::fs::read_to_string(dir.join("summary.json")) else {
                continue;
            };
            let Ok(trials_text) = std::fs::read_to_string(dir.join("trials.jsonl")) else {
                continue;
            };
            let summary: serde_json::Value = match serde_json::from_str(&summary_text) {
                Ok(v) => v,
                Err(_) => continue,
            };
            // Grand mean per composition_hash from this run's trials.
            let mut sums: BTreeMap<String, (f64, u64)> = BTreeMap::new();
            for line in trials_text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(t) = serde_json::from_str::<crate::Trial>(line) {
                    if t.composition_hash.is_empty() {
                        continue;
                    }
                    let e = sums.entry(t.composition_hash.clone()).or_insert((0.0, 0));
                    e.0 += t.reward;
                    e.1 += 1;
                }
            }
            let obj = summary.as_object().expect("summary is an object");
            for cfg in obj.values() {
                let (Some(hash), Some(recorded)) = (
                    cfg.get("composition_hash").and_then(|v| v.as_str()),
                    cfg.get("reward_mean").and_then(|v| v.as_f64()),
                ) else {
                    continue;
                };
                if let Some(&(sum, n)) = sums.get(hash) {
                    if n > 0 {
                        let grand = sum / n as f64;
                        assert!(
                            close(grand, recorded, 1e-3),
                            "run {:?} hash {hash}: computed {grand} vs summary {recorded}",
                            dir.file_name().unwrap()
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(
            checked > 0,
            "expected to reconcile at least one real config"
        );
    }
}
