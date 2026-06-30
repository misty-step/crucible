//! The read side of the eval: ingest real Daedalus arenas and runs into one
//! navigable [`Dataset`].
//!
//! Where [`crate::export`] writes Crucible's judgments *back* to Daedalus, this
//! module reads the *other* direction: a frozen tree of Daedalus arenas (the
//! answer keys) and runs (the trials a config produced) becomes a typed model an
//! eval dashboard can group, compare, and pool over. Six types, narrowest to
//! widest:
//!
//! - [`Trial`] — one line of a run's `trials.jsonl`: a single (config, task,
//!   trial) outcome, carrying the `reward` (continuous `0..=1`, partial credit
//!   like `0.8` is normal), the arena identity it was scored under, and the
//!   `composition_hash` of the config that produced it.
//! - [`Config`] — the unit of comparison, identified by `composition_hash`. A
//!   config's `id`/`kind` are *labels* that can drift; the **hash is identity**,
//!   so trials of one hash pool together even across runs that named it
//!   differently.
//! - [`EvalTask`] — one task in an arena: its id and the seeded [`Defect`]s from
//!   `tests/expected.json` (the scorer key), reusing [`crate::key`]'s loader.
//! - [`Eval`] — one **`(arena_id, arena_version)` group**: the arena's tasks and
//!   the configs that ran against it. This is the cell a leaderboard compares
//!   within, because a reward only means something against a fixed key.
//! - [`Run`] — a run *directory* on disk: its name, the arena identity its
//!   trials actually claim, and the configs it touched.
//! - [`Dataset`] — the whole ingest: every [`Eval`] group, every [`Run`], and an
//!   honest tally of everything skipped (a bad trial line, or a whole run
//!   candidate that placed nothing) with the [`SkipReason`] why.
//!
//! Two facts of the real corpus are load-bearing, and getting either wrong
//! silently corrupts every downstream number:
//!
//! 1. **The run directory name can lie about the arena.** A directory named
//!    `…-search-pr-review-v0` routinely holds trials whose `arena_id` is
//!    `pr-review-v2`. The directory name is a human label; `trials.jsonl`'s
//!    `arena_id` + `arena_version` are the ground truth. Every grouping decision
//!    here reads the trial, never the directory — [`Run`] keeps the directory
//!    name *and* the trials-derived arena precisely so the discrepancy stays
//!    visible rather than silently mis-binning a run.
//! 2. **A config's identity is its `composition_hash`, not its `id`/`kind`.**
//!    The same hash recurs across many runs (and many arenas); those trials are
//!    the *same* config and pool together within a group. The labels are carried
//!    for display, but identity, dedup, and pooling all key on the hash.
//!
//! The loader is **total**: [`Dataset::load`] never panics and never returns an
//! error. A malformed `trials.jsonl` line — invalid JSON, or valid JSON that
//! lacks the arena/hash identity needed to place it — is skipped and tallied
//! into [`Dataset::skipped`], so one corrupt line costs one trial, not the whole
//! ingest. The same honesty holds one level up, at whole inputs: a run candidate
//! that places no trial is never counted as a run, but it is never silently
//! dropped either — it lands in [`Dataset::skipped_inputs`] with a
//! [`SkipReason`]. That is what keeps the founding `pr-review-v0` runs — real,
//! scored trials that sit in top-level `*.jsonl` files and predate the
//! `composition_hash` identity — counted rather than vanishing before the tally
//! can see them. Unrecognized fields in real inputs are ignored, not rejected
//! (the same contract the rest of the crate keeps): the real records carry many
//! fields this model does not name, and adding one upstream must not break the
//! read.
//!
//! Once a corpus is ingested, the leaderboard layer *measures* it: a
//! [`Leaderboard`] ranks each group's configs by mean reward, attaches a
//! bootstrap interval to that continuous mean and a Wilson interval to the
//! binary solve rate, and
//! refuses (via McNemar plus a paired bootstrap) any rank gap that sits inside
//! the noise floor — reusing [`crate::measure`] throughout. That is the read
//! side's payoff: not just *what ran*, but *which config is defensibly better*.

mod leaderboard;

pub use leaderboard::{
    DeltaEstimate, DeltaSign, Estimate, Leaderboard, LeaderboardEntry, LeaderboardGroup,
    McnemarOutcome, Pairwise, PairwiseVerdict, Stronger,
};

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::key::{Defect, ExpectedKey};

/// One `trials.jsonl` record: a single (config, task, trial) outcome.
///
/// Deserialized leniently — every field defaults, so a sparse or partially
/// written line still loads, and unknown fields (the real records carry
/// `findings`, `artifacts`, token counts, …) are ignored. Identity is *not*
/// enforced at this layer: a trial with an empty [`arena_id`](Self::arena_id),
/// [`arena_version`](Self::arena_version), or
/// [`composition_hash`](Self::composition_hash) parses fine but is dropped at
/// placement time (and counted) by [`Dataset::load`], because it cannot be filed
/// under a group or a config.
///
/// `reward` is **continuous** in `0..=1`: the Daedalus scorer awards partial
/// credit (`reward = max(0, recall − 0.2·false_positives)`), so values like
/// `0.8` are ordinary and must not be rounded to a pass/fail bit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Trial {
    /// The record's own run id (e.g. `…-oracle-py-export-clear-t1`). This is the
    /// per-trial id from inside the file, **not** the run directory name; see
    /// [`Run::dir`] for the on-disk directory.
    pub run_id: String,
    /// Arena this trial was scored under — half of the group identity. The
    /// authoritative arena, regardless of what the enclosing directory is named.
    pub arena_id: String,
    /// Arena version this trial was scored under — the other half of the group
    /// identity. Historical trials reference versions the live `arena.toml` has
    /// already moved past, which is exactly why the version is read from the
    /// trial and never inferred from the arena directory.
    pub arena_version: String,
    /// Task within the arena this trial ran, e.g. `py-auth-sqli`.
    pub task_id: String,
    /// Trial index within the (config, task) cell (runs repeat trials for
    /// variance).
    pub trial: i64,
    /// Config label as recorded on this trial, e.g. `oracle`. A *label*: distinct
    /// trials of the same [`composition_hash`](Self::composition_hash) may carry
    /// different ids; the hash, not this, is identity.
    pub candidate_id: String,
    /// Config kind label, e.g. `oracle`, `null`, `pi`. A label, like
    /// [`candidate_id`](Self::candidate_id).
    pub candidate_kind: String,
    /// The config's content hash — its **stable identity**. Trials sharing this
    /// value are the same config and pool together within an [`Eval`].
    pub composition_hash: String,
    /// Model that produced the trial, when one was invoked. Absent for the
    /// deterministic `oracle`/`null` configs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Cost in USD, when known. Absent for offline/deterministic configs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Harness/runner error that aborted the trial before scoring, when any.
    /// Present (non-empty) marks an [error trial](Self::is_error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Wall-clock duration in milliseconds, when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wall_ms: Option<i64>,
    /// The trial's score: continuous in `0..=1`, partial credit included.
    pub reward: f64,
    /// Fraction of the task's expected defects this trial surfaced.
    pub recall: f64,
    /// Ids of the expected defects this trial matched.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub matched: Vec<String>,
    /// Count of surfaced findings that matched no expected defect — the penalty
    /// term in the reward.
    pub false_positives: i64,
    /// Number of defects the task's key seeded (the recall denominator).
    /// `None` on an [error trial](Self::is_error): the trial aborted before the
    /// scorer loaded the key, and the real records carry an explicit `null` here.
    /// It must be ingested (and the trial flagged), not skipped as malformed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_defects: Option<i64>,
    /// Scorer-side error (a key that failed to load, a malformed finding), when
    /// any. Also marks an [error trial](Self::is_error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorer_error: Option<String>,
}

impl Trial {
    /// Whether the trial failed rather than scored: a non-empty harness
    /// [`error`](Self::error) or [`scorer_error`](Self::scorer_error). Such a
    /// trial still carries a `reward` (typically `0.0`); callers that want a
    /// clean success rate exclude these.
    pub fn is_error(&self) -> bool {
        self.error.as_deref().is_some_and(|e| !e.is_empty())
            || self.scorer_error.as_deref().is_some_and(|e| !e.is_empty())
    }
}

/// One configuration under evaluation, identified by `composition_hash`.
///
/// The unit a leaderboard ranks. [`trials`](Self::trials) are pooled across every
/// run that produced this hash *within one [`Eval`] group* — pooling across
/// arenas would average rewards scored against different keys, so the grouping
/// stops at the `(arena_id, arena_version)` boundary even though the same hash
/// recurs in other groups. [`id`](Self::id) and [`kind`](Self::kind) are the
/// labels from the first trial seen for the hash; they are for display only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// The config's content hash — its identity within (and across) groups.
    pub composition_hash: String,
    /// Display label (the first trial's `candidate_id`). Not identity.
    pub id: String,
    /// Display kind (the first trial's `candidate_kind`). Not identity.
    pub kind: String,
    /// Every trial this config produced in the enclosing [`Eval`] group, pooled
    /// across runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trials: Vec<Trial>,
}

impl Config {
    /// How many trials pooled into this config.
    pub fn trial_count(&self) -> usize {
        self.trials.len()
    }

    /// How many of the pooled trials were [error trials](Trial::is_error).
    pub fn error_count(&self) -> usize {
        self.trials.iter().filter(|t| t.is_error()).count()
    }

    /// Mean reward over the pooled trials, or `None` when there are none.
    ///
    /// A plain arithmetic mean of the continuous rewards (error trials, which
    /// carry `reward == 0.0`, are included — exclude them upstream if a
    /// success-only mean is wanted). `None` rather than `0.0` for an empty
    /// config keeps "no data" distinct from "scored zero".
    pub fn reward_mean(&self) -> Option<f64> {
        if self.trials.is_empty() {
            return None;
        }
        let sum: f64 = self.trials.iter().map(|t| t.reward).sum();
        Some(sum / self.trials.len() as f64)
    }
}

/// One task in an arena: its id and the seeded defects it is scored against.
///
/// [`defects`](Self::defects) come from the task's `tests/expected.json` (the
/// span key `daedalus-score` reads), loaded via [`ExpectedKey`]. The list is
/// empty when that file is absent or unreadable for the resolved arena directory
/// — a historical `(arena_id, arena_version)` group whose key has since moved on
/// still names its tasks, just without recoverable defect rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalTask {
    /// Task id, e.g. `py-auth-sqli`.
    pub id: String,
    /// Seeded defects from `tests/expected.json`; empty when none could be read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defects: Vec<Defect>,
}

/// One `(arena_id, arena_version)` group: the cell a reward is comparable within.
///
/// The identity is derived from the trials, never from a directory name. Holds
/// the arena's [`tasks`](Self::tasks) (those any trial in the group exercised,
/// each enriched with its key's defects when available) and the
/// [`configs`](Self::configs) that ran against this exact arena version, sorted
/// by `composition_hash`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Eval {
    /// Arena id for the group, from the trials.
    pub arena_id: String,
    /// Arena version for the group, from the trials.
    pub arena_version: String,
    /// Tasks exercised by the group's trials, sorted by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<EvalTask>,
    /// Configs that ran against this arena version, sorted by `composition_hash`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub configs: Vec<Config>,
}

impl Eval {
    /// The config with this `composition_hash`, if it ran in this group.
    pub fn config(&self, composition_hash: &str) -> Option<&Config> {
        self.configs
            .iter()
            .find(|c| c.composition_hash == composition_hash)
    }

    /// Total trials across every config in the group.
    pub fn trial_count(&self) -> usize {
        self.configs.iter().map(Config::trial_count).sum()
    }
}

/// One run *directory* on disk, as a provenance record.
///
/// Carries the directory's [`dir`](Self::dir) name *and* the
/// [`arena_id`](Self::arena_id)/[`arena_version`](Self::arena_version) its trials
/// actually claim, so the routine "the directory name lies about the arena"
/// discrepancy is inspectable rather than buried. The arena identity is the
/// dominant `(arena_id, arena_version)` among the directory's placeable trials
/// (in the real corpus a directory holds exactly one). A run owns no trials here
/// — those live under the [`Config`] they pool into; this is the read receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// The run directory's name on disk (the human label, which may disagree with
    /// [`arena_id`](Self::arena_id)).
    pub dir: String,
    /// Arena id its trials claim — the truth, read from `trials.jsonl`.
    pub arena_id: String,
    /// Arena version its trials claim.
    pub arena_version: String,
    /// Count of placeable trials read from the directory.
    pub trial_count: usize,
    /// Distinct `composition_hash`es the directory touched, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_hashes: Vec<String>,
}

/// Why a run candidate under `runs_dir` was read but not ingested as a [`Run`].
///
/// The loader treats two things as run candidates: a subdirectory (read for its
/// `trials.jsonl`) and a top-level `*.jsonl` file (the founding pre-hash runs sit
/// loose in `runs_dir`, not in a subdirectory). A candidate that yields no
/// placeable trial is **not** counted as a run — that would inflate the run total
/// with a phantom — but it is never silently dropped either: it is recorded in
/// [`Dataset::skipped_inputs`] with one of these reasons, so the ingest stays
/// honest about what it declined and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// A directory with no `trials.jsonl` but a sibling `score.json` — the older
    /// per-task Daedalus scoring format this v0 loader does not read. Real scored
    /// work, in a shape this model cannot place; left out deliberately and counted
    /// so the omission is visible rather than silent.
    UnsupportedFormat,
    /// A directory with neither a `trials.jsonl` nor a recognized alternative
    /// (e.g. a bare regression-command note): no scored-trial source at all.
    NoTrialsFile,
    /// A trials source was read — a directory's `trials.jsonl`, or a top-level
    /// `*.jsonl` — but every line was blank, unparseable, or lacked the
    /// arena/hash identity needed to place it, so the candidate contributed zero
    /// trials to any group. The founding `pr-review-v0` files land here: real,
    /// scored trials that predate `composition_hash` and so cannot be filed under
    /// a hash-keyed [`Config`]. The skipped lines are also counted in
    /// [`Dataset::skipped`]; [`SkippedInput::trials`] records how many this input
    /// carried.
    NoPlaceableTrials,
}

/// One run candidate that was read but not ingested as a [`Run`], with the
/// [`reason`](Self::reason) it was declined.
///
/// A health-and-provenance record: it makes the loader's run total trustworthy
/// (only real runs are counted) while guaranteeing nothing a human dropped into
/// `runs_dir` vanishes unaccounted. Listed in [`Dataset::skipped_inputs`] in the
/// loader's sorted-name walk order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedInput {
    /// The directory or file name under `runs_dir`.
    pub name: String,
    /// Why it was not ingested as a run.
    pub reason: SkipReason,
    /// Trial lines this input carried, all skipped because none were placeable
    /// (also counted in [`Dataset::skipped`]). `0` for an input with no trials
    /// source at all ([`UnsupportedFormat`](SkipReason::UnsupportedFormat) or
    /// [`NoTrialsFile`](SkipReason::NoTrialsFile)).
    pub trials: usize,
}

/// The whole ingest: every group, every run, and an honest account of what was
/// skipped.
///
/// Built by [`load`](Self::load). [`evals`](Self::evals) is the set of
/// `(arena_id, arena_version)` groups (sorted by that key); over the real corpus
/// this is ~12. The two skip tallies are health signals, not fatal errors, and
/// they answer different questions: [`skipped`](Self::skipped) counts trial
/// *lines* that could not be parsed or placed, while
/// [`skipped_inputs`](Self::skipped_inputs) lists whole run *candidates* (a
/// directory or a top-level file) read but not counted as a run. Every candidate
/// becomes exactly one of a [`Run`] in [`runs`](Self::runs) or a
/// [`SkippedInput`] in [`skipped_inputs`](Self::skipped_inputs); none vanishes
/// unaccounted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Dataset {
    /// The `(arena_id, arena_version)` groups, sorted by that key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evals: Vec<Eval>,
    /// One record per run directory or top-level `*.jsonl` that placed at least
    /// one trial, sorted by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<Run>,
    /// Count of trial lines skipped: invalid JSON, or valid JSON missing the
    /// arena/hash identity needed to place it. Counts lines from top-level
    /// `*.jsonl` candidates too, so a founding pre-hash trial is counted here, not
    /// dropped before the tally can see it.
    #[serde(default)]
    pub skipped: usize,
    /// Run candidates read but not ingested as runs, each with its
    /// [`SkipReason`]. Distinct from [`skipped`](Self::skipped): that counts trial
    /// *lines*; this counts whole *inputs* (a directory or a top-level file) the
    /// loader declined to count as a run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_inputs: Vec<SkippedInput>,
}

/// Accumulator for one `(arena_id, arena_version)` group while loading.
#[derive(Default)]
struct GroupAccumulator {
    /// Configs keyed by `composition_hash`, so trials of one hash pool regardless
    /// of which run or label they arrived under.
    configs: BTreeMap<String, Config>,
    /// Task ids any trial in the group exercised, deduplicated and sorted.
    task_ids: BTreeSet<String>,
}

impl Dataset {
    /// Load every arena and run under `arenas_dir` and `runs_dir` into a
    /// [`Dataset`]. Total: never panics, never errors.
    ///
    /// Walks `runs_dir` for run candidates — each subdirectory (read for its
    /// `trials.jsonl`) and each top-level `*.jsonl` file (the founding pre-hash
    /// runs sit loose, not in a subdirectory) — parses each line by line, and
    /// files every placeable trial under its `(arena_id, arena_version)` group and
    /// `composition_hash` config, reading both identities from the trial, never
    /// the directory name. Each group's task keys are then loaded from
    /// `arenas_dir/<arena_id>/tasks/<task_id>/tests/expected.json`.
    ///
    /// Nothing is dropped uncounted. A line that fails to parse, or that lacks
    /// arena or hash identity, is skipped and counted in [`skipped`](Self::skipped).
    /// A candidate that places **no** trial is not counted as a run — that would
    /// inflate the run total with a phantom whose arena is empty — but is recorded
    /// in [`skipped_inputs`](Self::skipped_inputs) with a [`SkipReason`]: a
    /// directory holding a `score.json` instead of a `trials.jsonl` as
    /// [`UnsupportedFormat`](SkipReason::UnsupportedFormat), one holding neither as
    /// [`NoTrialsFile`](SkipReason::NoTrialsFile), and a read-but-unplaceable
    /// trials source (an all-blank `trials.jsonl`, or a founding `*.jsonl` whose
    /// trials predate `composition_hash`) as
    /// [`NoPlaceableTrials`](SkipReason::NoPlaceableTrials). A top-level file that
    /// is not `*.jsonl` is not a run candidate and is ignored. An unreadable
    /// `runs_dir` yields an empty dataset.
    ///
    /// Candidates are processed in sorted name order, so the result — including
    /// each config's first-seen `id`/`kind` labels, the run list, and the
    /// skipped-input list — is deterministic.
    pub fn load(arenas_dir: impl AsRef<Path>, runs_dir: impl AsRef<Path>) -> Self {
        let arenas_dir = arenas_dir.as_ref();
        let runs_dir = runs_dir.as_ref();

        let mut entries: Vec<_> = match std::fs::read_dir(runs_dir) {
            Ok(read_dir) => read_dir.flatten().collect(),
            Err(_) => return Dataset::default(),
        };
        entries.sort_by_key(std::fs::DirEntry::file_name);

        let mut groups: BTreeMap<(String, String), GroupAccumulator> = BTreeMap::new();
        let mut runs: Vec<Run> = Vec::new();
        let mut skipped: usize = 0;
        let mut skipped_inputs: Vec<SkippedInput> = Vec::new();

        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();

            // Resolve the candidate's trials source. A subdirectory is read for
            // its `trials.jsonl`; a top-level `*.jsonl` file is itself the source
            // (the founding pre-hash runs sit loose in `runs_dir`). Anything else
            // — a stray note, a README — is not a run candidate.
            let content = if path.is_dir() {
                match std::fs::read_to_string(path.join("trials.jsonl")) {
                    Ok(content) => content,
                    Err(_) => {
                        // No trials.jsonl: classify and count, never silently drop.
                        // A sibling `score.json` marks the older per-task format
                        // this loader does not read; otherwise there is no source.
                        let reason = if path.join("score.json").is_file() {
                            SkipReason::UnsupportedFormat
                        } else {
                            SkipReason::NoTrialsFile
                        };
                        skipped_inputs.push(SkippedInput {
                            name,
                            reason,
                            trials: 0,
                        });
                        continue;
                    }
                }
            } else if is_jsonl(&path) {
                match std::fs::read_to_string(&path) {
                    Ok(content) => content,
                    // Unreadable top-level file: no recoverable trials to count.
                    Err(_) => continue,
                }
            } else {
                continue;
            };

            let mut run_group_counts: BTreeMap<(String, String), usize> = BTreeMap::new();
            let mut run_hashes: BTreeSet<String> = BTreeSet::new();
            let mut run_trial_count: usize = 0;
            let mut input_skipped: usize = 0;

            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let trial: Trial = match serde_json::from_str(line) {
                    Ok(trial) => trial,
                    Err(_) => {
                        skipped += 1;
                        input_skipped += 1;
                        continue;
                    }
                };
                if trial.arena_id.is_empty()
                    || trial.arena_version.is_empty()
                    || trial.composition_hash.is_empty()
                {
                    skipped += 1;
                    input_skipped += 1;
                    continue;
                }

                let key = (trial.arena_id.clone(), trial.arena_version.clone());
                *run_group_counts.entry(key.clone()).or_default() += 1;
                run_hashes.insert(trial.composition_hash.clone());
                run_trial_count += 1;

                let group = groups.entry(key).or_default();
                group.task_ids.insert(trial.task_id.clone());
                group
                    .configs
                    .entry(trial.composition_hash.clone())
                    .or_insert_with(|| Config {
                        composition_hash: trial.composition_hash.clone(),
                        id: trial.candidate_id.clone(),
                        kind: trial.candidate_kind.clone(),
                        trials: Vec::new(),
                    })
                    .trials
                    .push(trial);
            }

            // A candidate that placed no trial is not a run — counting it would
            // inflate the run total with a phantom whose arena is empty. Record it
            // (with the count of trial lines it carried and skipped) so the
            // omission is visible rather than silent.
            if run_trial_count == 0 {
                skipped_inputs.push(SkippedInput {
                    name,
                    reason: SkipReason::NoPlaceableTrials,
                    trials: input_skipped,
                });
                continue;
            }

            let (arena_id, arena_version) = dominant_group(&run_group_counts);
            runs.push(Run {
                dir: name,
                arena_id,
                arena_version,
                trial_count: run_trial_count,
                config_hashes: run_hashes.into_iter().collect(),
            });
        }

        let evals = groups
            .into_iter()
            .map(|((arena_id, arena_version), group)| {
                let tasks = group
                    .task_ids
                    .iter()
                    .map(|task_id| EvalTask {
                        id: task_id.clone(),
                        defects: load_defects(arenas_dir, &arena_id, task_id),
                    })
                    .collect();
                Eval {
                    arena_id,
                    arena_version,
                    tasks,
                    configs: group.configs.into_values().collect(),
                }
            })
            .collect();

        Dataset {
            evals,
            runs,
            skipped,
            skipped_inputs,
        }
    }

    /// The group for this `(arena_id, arena_version)`, if present.
    pub fn eval(&self, arena_id: &str, arena_version: &str) -> Option<&Eval> {
        self.evals
            .iter()
            .find(|e| e.arena_id == arena_id && e.arena_version == arena_version)
    }

    /// Number of `(arena_id, arena_version)` groups — the headline cardinality.
    pub fn group_count(&self) -> usize {
        self.evals.len()
    }

    /// Total placeable trials across every group.
    pub fn trial_count(&self) -> usize {
        self.evals.iter().map(Eval::trial_count).sum()
    }
}

/// The most-trialed `(arena_id, arena_version)` in a directory, or empty when the
/// directory yielded no placeable trial. Ties resolve to the largest key, so the
/// choice is deterministic; the real corpus never mixes arenas in one directory.
fn dominant_group(counts: &BTreeMap<(String, String), usize>) -> (String, String) {
    counts
        .iter()
        .max_by_key(|entry| *entry.1)
        .map(|(key, _)| key.clone())
        .unwrap_or_default()
}

/// Whether `path` is a top-level `*.jsonl` trials file — a founding run that
/// lives loose in `runs_dir` rather than inside its own directory.
fn is_jsonl(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
}

/// Load a task's seeded defects from `arenas_dir/<arena_id>/tasks/<task_id>/
/// tests/expected.json`, returning an empty vec when the key is absent or
/// unreadable (a missing key must not abort the ingest).
fn load_defects(arenas_dir: &Path, arena_id: &str, task_id: &str) -> Vec<Defect> {
    let path = arenas_dir
        .join(arena_id)
        .join("tasks")
        .join(task_id)
        .join("tests")
        .join("expected.json");
    match ExpectedKey::from_path(&path) {
        Ok(key) => key.defects,
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A self-deleting scratch tree under the OS temp dir, so a synthetic
    /// arenas/runs fixture can be built on disk and exercised through the real
    /// [`Dataset::load`] path with no committed fixtures and no leftovers.
    struct TempTree {
        root: PathBuf,
    }

    impl TempTree {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "crucible-dashboard-test-{tag}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&root).expect("create scratch root");
            Self { root }
        }

        /// Write `contents` to `rel` under the tree, creating parent dirs.
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

    /// A single `trials.jsonl` line with the given arena/version/hash/label/reward.
    fn trial_line(
        arena: &str,
        version: &str,
        hash: &str,
        candidate_id: &str,
        task: &str,
        reward: f64,
    ) -> String {
        format!(
            r#"{{"run_id":"r","arena_id":"{arena}","arena_version":"{version}","task_id":"{task}","trial":1,"candidate_id":"{candidate_id}","candidate_kind":"k","composition_hash":"{hash}","model":null,"cost_usd":null,"error":null,"wall_ms":12,"reward":{reward},"recall":{reward},"matched":[],"false_positives":0,"expected_defects":1,"scorer_error":null}}"#
        )
    }

    /// A founding-style record: real candidate labels and a reward, but **no**
    /// `composition_hash` — these predate the hash identity. It parses fine, yet
    /// is unplaceable, exactly like the real top-level `pr-review-v0` files.
    fn founding_line(
        arena: &str,
        version: &str,
        candidate_id: &str,
        task: &str,
        reward: f64,
    ) -> String {
        format!(
            r#"{{"run_id":"r","arena_id":"{arena}","arena_version":"{version}","task_id":"{task}","trial":1,"candidate_id":"{candidate_id}","candidate_kind":"oracle","reward":{reward},"recall":{reward},"matched":[],"false_positives":0,"expected_defects":1}}"#
        )
    }

    #[test]
    fn trial_deserializes_real_shape_and_ignores_unknown_fields() {
        // Verbatim-shaped record with extra fields the model does not name
        // (taskspec, findings, artifacts, tokens_*) — they must be ignored, not
        // rejected, and the named fields must read through.
        let line = r#"{"run_id":"20260620T170801Z-oracle-py-export-clear-t1","ts_start":"2026-06-20T17:08:01+00:00","arena_id":"pr-review-correctness-v0","arena_version":"0.3.0","taskspec":"specs/x/taskspec.toml","task_id":"py-export-clear","trial":1,"candidate_id":"oracle","candidate_kind":"oracle","composition_hash":"07d8650e238cd916","model":null,"cost_usd":null,"error":null,"wall_ms":18,"reward":0.8,"recall":0.8,"matched":["py-export-clear"],"false_positives":0,"expected_defects":1,"scorer_error":null,"findings":[{"file":"a","line":1}],"artifacts":"artifacts/oracle/x","tokens_prompt":null}"#;
        let trial: Trial = serde_json::from_str(line).expect("real shape must parse");
        assert_eq!(trial.arena_id, "pr-review-correctness-v0");
        assert_eq!(trial.arena_version, "0.3.0");
        assert_eq!(trial.composition_hash, "07d8650e238cd916");
        assert_eq!(trial.task_id, "py-export-clear");
        assert_eq!(trial.reward, 0.8, "reward is continuous, not rounded");
        assert_eq!(trial.matched, vec!["py-export-clear".to_string()]);
        assert!(trial.model.is_none());
        assert!(
            !trial.is_error(),
            "a null error/scorer_error is not an error"
        );
    }

    #[test]
    fn error_trial_is_flagged() {
        let mut trial: Trial =
            serde_json::from_str(&trial_line("a", "1", "h", "c", "t", 0.0)).unwrap();
        assert!(!trial.is_error());
        trial.scorer_error = Some("key failed to load".to_string());
        assert!(trial.is_error(), "a scorer_error marks the trial failed");
        trial.scorer_error = Some(String::new());
        assert!(!trial.is_error(), "an empty error string is not an error");
    }

    #[test]
    fn error_trial_with_null_expected_defects_is_ingested_not_skipped() {
        // Real error trials abort before scoring and record `expected_defects:
        // null` with a non-null `error`. They are valid data to ingest and flag,
        // never malformed input to skip — a regression guard for the gate, since
        // the live-corpus check is `#[ignore]`d.
        let tree = TempTree::new("errtrial");
        let errored = r#"{"run_id":"r","arena_id":"pr-review-v1","arena_version":"0.1.0","task_id":"discount-after-tax","trial":2,"candidate_id":"baseline-oneshot","candidate_kind":"oneshot","composition_hash":"546acf97c8be1b1a","model":"some/model","cost_usd":0.028,"error":"no JSON object found in model output","wall_ms":285401,"reward":0.0,"recall":0.0,"matched":[],"false_positives":0,"expected_defects":null,"scorer_error":null}"#;
        tree.write("runs/r/trials.jsonl", &format!("{errored}\n"));

        let ds = Dataset::load(tree.arenas(), tree.runs());

        assert_eq!(ds.skipped, 0, "an error trial is not malformed");
        assert_eq!(ds.trial_count(), 1, "the error trial is ingested");
        let config = ds
            .eval("pr-review-v1", "0.1.0")
            .and_then(|e| e.config("546acf97c8be1b1a"))
            .expect("config placed");
        assert_eq!(config.error_count(), 1, "and flagged as an error");
        assert_eq!(config.trials[0].expected_defects, None);
    }

    #[test]
    fn arena_identity_comes_from_trials_not_the_directory_name() {
        // Trap 1: a directory named for pr-review-v0 whose trials are actually
        // pr-review-v2/0.2.0. The group must key on the trial, and the Run must
        // expose both the (misleading) directory name and the true arena.
        let tree = TempTree::new("trap1");
        tree.write(
            "runs/20260610T160533Z-search-pr-review-v0/trials.jsonl",
            &trial_line("pr-review-v2", "0.2.0", "abc123", "search", "task-a", 1.0),
        );

        let ds = Dataset::load(tree.arenas(), tree.runs());

        assert_eq!(ds.group_count(), 1);
        assert!(
            ds.eval("pr-review-v2", "0.2.0").is_some(),
            "grouped under the trial's arena"
        );
        assert!(
            ds.eval("pr-review-v0", "0.2.0").is_none(),
            "never grouped under the directory-name arena"
        );
        let run = &ds.runs[0];
        assert_eq!(run.dir, "20260610T160533Z-search-pr-review-v0");
        assert_eq!(
            run.arena_id, "pr-review-v2",
            "the run's true arena is the trials' arena, not its directory name"
        );
        assert_eq!(run.arena_version, "0.2.0");
    }

    #[test]
    fn config_identity_is_the_hash_pooled_across_runs_and_labels() {
        // Trap 2: one composition_hash recorded under two different ids across two
        // runs is ONE config; its trials pool. Directories sort by name, so the
        // first-seen label is deterministic ("run-a" before "run-b" => "alpha").
        let tree = TempTree::new("trap2");
        tree.write(
            "runs/run-a/trials.jsonl",
            &trial_line("arena-x", "0.1.0", "sharedhash", "alpha", "t1", 0.8),
        );
        tree.write(
            "runs/run-b/trials.jsonl",
            &trial_line("arena-x", "0.1.0", "sharedhash", "beta", "t1", 1.0),
        );

        let ds = Dataset::load(tree.arenas(), tree.runs());

        let eval = ds.eval("arena-x", "0.1.0").expect("group exists");
        assert_eq!(eval.configs.len(), 1, "one hash is one config");
        let config = eval.config("sharedhash").expect("config by hash");
        assert_eq!(config.trial_count(), 2, "trials pooled across both runs");
        assert_eq!(
            config.id, "alpha",
            "first-seen label wins (run-a sorts before run-b)"
        );
        let mean = config.reward_mean().expect("non-empty config has a mean");
        assert!(
            (mean - 0.9).abs() < 1e-9,
            "continuous reward mean of 0.8 and 1.0 is 0.9, got {mean}"
        );
        assert_eq!(config.error_count(), 0);
        assert_eq!(ds.runs.len(), 2, "both run directories recorded");
    }

    #[test]
    fn malformed_lines_are_skipped_and_counted_never_fatal() {
        let tree = TempTree::new("skips");
        // One placeable trial, then four unplaceable inputs and a blank line.
        let body = format!(
            "{valid}\nthis is not json\n{empty_hash}\n{empty_arena}\n\n{{\"truncated\":\n",
            valid = trial_line("arena-x", "0.1.0", "h1", "c", "t1", 1.0),
            empty_hash = r#"{"arena_id":"arena-x","arena_version":"0.1.0","composition_hash":""}"#,
            empty_arena = r#"{"arena_id":"","arena_version":"0.1.0","composition_hash":"h2"}"#,
        );
        tree.write("runs/r/trials.jsonl", &body);

        let ds = Dataset::load(tree.arenas(), tree.runs());

        assert_eq!(
            ds.skipped, 4,
            "non-JSON, empty-hash, empty-arena, and truncated lines skip; blank does not"
        );
        assert_eq!(ds.trial_count(), 1, "the one valid trial still loads");
        assert_eq!(ds.group_count(), 1);
        assert_eq!(ds.runs[0].trial_count, 1);
    }

    #[test]
    fn tasks_load_defects_from_expected_json_and_tolerate_absence() {
        let tree = TempTree::new("tasks");
        // t1 has a real expected.json (one defect); t2 is exercised by a trial but
        // has no key on disk — it is still listed, with no defects.
        tree.write(
            "arenas/arena-x/tasks/t1/tests/expected.json",
            r#"{"defects":[{"id":"sqli","file":"app/auth.py","line_start":8,"line_end":12,"category":"security","note":"interpolated email"}]}"#,
        );
        let body = format!(
            "{}\n{}\n",
            trial_line("arena-x", "0.1.0", "h1", "c", "t1", 1.0),
            trial_line("arena-x", "0.1.0", "h1", "c", "t2", 0.0),
        );
        tree.write("runs/r/trials.jsonl", &body);

        let ds = Dataset::load(tree.arenas(), tree.runs());
        let eval = ds.eval("arena-x", "0.1.0").expect("group exists");

        assert_eq!(
            eval.tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["t1", "t2"],
            "tasks are sorted by id"
        );
        let t1 = &eval.tasks[0];
        assert_eq!(t1.defects.len(), 1, "t1's seeded defect loads");
        assert_eq!(t1.defects[0].id, "sqli");
        assert_eq!(t1.defects[0].category, "security");
        assert!(
            eval.tasks[1].defects.is_empty(),
            "a task with no key on disk is listed without defects"
        );
    }

    #[test]
    fn synthetic_fixture_groups_pools_and_counts() {
        // The headline synthetic fixture: three run directories across two
        // (arena,version) groups, one hash shared across two runs of arena-x, a
        // separate arena-y run, plus a malformed line. Exercises grouping,
        // pooling, the skip count, and the run receipts end to end.
        let tree = TempTree::new("synthetic");
        tree.write(
            "arenas/arena-x/tasks/t1/tests/expected.json",
            r#"{"defects":[{"id":"d1","file":"f.py","line_start":1,"line_end":2,"category":"security"}]}"#,
        );
        tree.write(
            "runs/x-run-1/trials.jsonl",
            &format!(
                "{}\ngarbage\n",
                trial_line("arena-x", "0.1.0", "hx", "cfg-x", "t1", 0.8)
            ),
        );
        tree.write(
            "runs/x-run-2/trials.jsonl",
            &trial_line("arena-x", "0.1.0", "hx", "cfg-x", "t1", 1.0),
        );
        tree.write(
            "runs/y-run-1/trials.jsonl",
            &trial_line("arena-y", "0.2.0", "hy", "cfg-y", "t9", 0.5),
        );

        let ds = Dataset::load(tree.arenas(), tree.runs());

        assert_eq!(ds.group_count(), 2, "two (arena,version) groups");
        assert_eq!(ds.skipped, 1, "the one garbage line is counted");
        assert_eq!(ds.runs.len(), 3, "three run directories");
        assert_eq!(ds.trial_count(), 3, "three placeable trials");
        assert!(
            ds.skipped_inputs.is_empty(),
            "well-formed runs produce no skipped inputs"
        );

        let x = ds.eval("arena-x", "0.1.0").expect("arena-x group");
        assert_eq!(x.configs.len(), 1, "arena-x has one pooled config");
        assert_eq!(
            x.config("hx").unwrap().trial_count(),
            2,
            "pooled across runs"
        );
        assert_eq!(x.tasks.len(), 1);
        assert_eq!(x.tasks[0].defects.len(), 1, "arena-x task key loaded");

        let y = ds.eval("arena-y", "0.2.0").expect("arena-y group");
        assert_eq!(y.config("hy").unwrap().reward_mean(), Some(0.5));

        // Evals and runs are deterministically ordered.
        assert_eq!(
            ds.evals
                .iter()
                .map(|e| (e.arena_id.as_str(), e.arena_version.as_str()))
                .collect::<Vec<_>>(),
            vec![("arena-x", "0.1.0"), ("arena-y", "0.2.0")]
        );
        assert_eq!(
            ds.runs.iter().map(|r| r.dir.as_str()).collect::<Vec<_>>(),
            vec!["x-run-1", "x-run-2", "y-run-1"]
        );
    }

    #[test]
    fn top_level_jsonl_and_unplaceable_inputs_are_counted_never_silently_dropped() {
        // The INGEST-honesty fixture, covering all three findings at once:
        //   1. a top-level `*.jsonl` founding run (no composition_hash) whose
        //      trials used to be dropped BEFORE the skip counter, uncounted;
        //   2. a run whose trials.jsonl places nothing, which used to be pushed as
        //      a phantom run with an empty arena and counted in "N runs";
        //   3. a score.json directory (the older per-task format) that used to be
        //      silently ignored.
        // After the fix every candidate becomes exactly one run or one counted
        // skipped input, and a stray non-jsonl file is left alone.
        let tree = TempTree::new("honesty");

        // (1) two founding trials, both unplaceable (no composition_hash).
        tree.write(
            "runs/00-founding-oracle.jsonl",
            &format!(
                "{}\n{}\n",
                founding_line("pr-review-v0", "0.1.0", "oracle", "js-cart-total", 1.0),
                founding_line("pr-review-v0", "0.1.1", "null", "py-auth-sqli", 0.0),
            ),
        );
        // (2) an all-blank trials.jsonl: read, but zero placeable.
        tree.write("runs/empty-run/trials.jsonl", "\n  \n\n");
        // (3) the older per-task format: score.json, no trials.jsonl.
        tree.write("runs/unsupported-dir/score.json", r#"{"reward":1.0}"#);
        // a directory with no recognized trials source at all.
        tree.write("runs/no-trials-dir/regression-command.txt", "rerun me");
        // one real run so the dataset is not degenerate.
        tree.write(
            "runs/normal-run/trials.jsonl",
            &trial_line("arena-x", "0.1.0", "h1", "cfg", "t1", 0.8),
        );
        // a stray non-jsonl top-level file: not a run candidate, must be ignored.
        tree.write("runs/NOTEBOOK.md", "# notes\n");

        let ds = Dataset::load(tree.arenas(), tree.runs());

        // Exactly one real run; the phantom empty run is NOT counted.
        assert_eq!(ds.runs.len(), 1, "only the placeable run is a run");
        assert_eq!(ds.runs[0].dir, "normal-run");
        assert!(
            !ds.runs[0].arena_id.is_empty(),
            "a counted run has a real arena"
        );
        assert_eq!(ds.trial_count(), 1);

        // The two founding lines are counted in the trial-line tally, not dropped.
        assert_eq!(ds.skipped, 2, "both hashless founding lines are counted");

        // Every declined candidate is accounted for, with its reason, in the
        // loader's sorted-name walk order.
        let got: Vec<(&str, SkipReason, usize)> = ds
            .skipped_inputs
            .iter()
            .map(|s| (s.name.as_str(), s.reason, s.trials))
            .collect();
        assert_eq!(
            got,
            vec![
                ("00-founding-oracle.jsonl", SkipReason::NoPlaceableTrials, 2),
                ("empty-run", SkipReason::NoPlaceableTrials, 0),
                ("no-trials-dir", SkipReason::NoTrialsFile, 0),
                ("unsupported-dir", SkipReason::UnsupportedFormat, 0),
            ],
            "founding file + empty run + no-trials + unsupported dir each counted once"
        );
        // The stray markdown file is not a run candidate and is not skip-counted.
        assert!(
            ds.skipped_inputs.iter().all(|s| s.name != "NOTEBOOK.md"),
            "a non-jsonl top-level file is ignored, not recorded"
        );
    }

    #[test]
    fn reward_mean_is_none_for_an_empty_config() {
        let config = Config {
            composition_hash: "h".to_string(),
            id: "c".to_string(),
            kind: "k".to_string(),
            trials: Vec::new(),
        };
        assert_eq!(config.reward_mean(), None, "no trials is not a zero score");
        assert_eq!(config.trial_count(), 0);
    }

    #[test]
    fn unreadable_runs_dir_yields_an_empty_dataset() {
        let tree = TempTree::new("missing");
        // runs/ was never created under this tree.
        let ds = Dataset::load(tree.arenas(), tree.runs());
        assert_eq!(ds.group_count(), 0);
        assert!(ds.runs.is_empty());
        assert_eq!(ds.skipped, 0);
    }

    /// Real-data honesty harness: against a local Daedalus checkout
    /// (`CRUCIBLE_DAEDALUS_DIR`), the corpus collapses to ~12 groups, every run
    /// counted is a real run, and nothing skipped vanishes uncounted. The 37
    /// founding `pr-review-v0` trials live in top-level `*.jsonl` files and predate
    /// `composition_hash`; they must surface in the skip tallies, not be dropped
    /// before counting. Ignored by default so the gate never depends on that
    /// checkout; run with `cargo test -p crucible-core -- --ignored` and the env
    /// var set.
    #[test]
    #[ignore = "requires a local Daedalus checkout via CRUCIBLE_DAEDALUS_DIR"]
    fn real_daedalus_corpus_groups_and_accounts_for_every_input() {
        let Ok(root) = std::env::var("CRUCIBLE_DAEDALUS_DIR") else {
            return;
        };
        let root = Path::new(&root);
        let ds = Dataset::load(root.join("arenas"), root.join("runs"));
        let groups: Vec<_> = ds
            .evals
            .iter()
            .map(|e| (e.arena_id.clone(), e.arena_version.clone()))
            .collect();
        assert!(
            (10..=14).contains(&ds.group_count()),
            "expected ~12 (arena,version) groups, got {}: {groups:?}",
            ds.group_count()
        );
        assert!(
            ds.evals.iter().any(|e| e.trial_count() > 0),
            "groups carry pooled trials"
        );

        // No phantom runs: every counted run placed at least one trial under a
        // real arena (the empty-arena run that used to be pushed is gone).
        assert!(
            ds.runs
                .iter()
                .all(|r| r.trial_count > 0 && !r.arena_id.is_empty()),
            "a counted run must have placeable trials and a real arena"
        );

        // The 37 founding trials are now counted, not silently dropped: they are
        // unplaceable (no composition_hash), so they surface as NoPlaceableTrials
        // inputs whose carried lines are also tallied in `skipped`.
        let founding: usize = ds
            .skipped_inputs
            .iter()
            .filter(|s| s.reason == SkipReason::NoPlaceableTrials)
            .map(|s| s.trials)
            .sum();
        assert_eq!(
            founding, 37,
            "the 37 founding pr-review-v0 trials are counted as skipped inputs"
        );
        assert!(
            ds.skipped >= 37,
            "founding lines are counted in the trial-line tally too, got {}",
            ds.skipped
        );

        // The cerberus-rd-lab score.json dirs are declined as unsupported format,
        // not silently ignored.
        assert!(
            ds.skipped_inputs
                .iter()
                .any(|s| s.reason == SkipReason::UnsupportedFormat),
            "score.json dirs are counted as unsupported-format, not ignored"
        );
    }
}
