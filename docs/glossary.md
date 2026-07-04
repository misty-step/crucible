# Glossary

Plain-language definitions of the Crucible-specific terms that show up in the
CLI output, the README, `SKILL.md`, and the docs. Each entry links to where
the concept is actually implemented, so "what does this mean" and "where does
it live in the code" are the same lookup.

If you just want to run something, start with
[`docs/operator-walkthrough.md`](operator-walkthrough.md) instead — you can
follow that path without knowing any of these terms up front, and come back
here when the output uses a word you don't recognize.

- **EvalSpec** — the JSON document that declares one benchmark: what it
  measures, how candidate output is graded, and which runner executes it. Free
  text (`task`, `inputs`, `outputs`, `decision`) is for humans and models;
  `graders`, `aggregation`, and `uncertainty` are the rigid fields the runner
  actually branches on. Every file under `evals/*.json` is one EvalSpec.
  Defined in `crucible-core/src/spec.rs` (`struct EvalSpec`).

- **Runner kind** — which execution family runs an EvalSpec's tasks. There are
  three: `key_recall` (score review findings against expected PR-review rows),
  `prompt_benchmark` (send prompts to a live OpenRouter model and grade the
  text with a deterministic rubric), and `agentic_judge` (grade a candidate
  with a live model judge instead of a fixed rubric). It's a closed enum, not
  a plugin system — adding a runner kind means adding a real execution path in
  Crucible. Defined in `crucible-core/src/spec.rs` (`enum RunnerKind`).

- **Wilson interval** — the confidence interval Crucible reports around every
  binary pass rate (`lower`/`upper` next to `point` in run output), computed
  with the Wilson score method rather than a naive normal approximation, which
  gets exploitably wrong at small sample sizes. This is why a 4/5 or 5/5 score
  on a five-task benchmark still shows a wide interval — five tasks just isn't
  enough evidence to narrow it. Implemented in
  `crucible-core/src/measure/rate.rs` (`fn wilson_interval`), applied via
  `crucible/src/main.rs` (`fn wilson_score`).

- **Noise floor** — the standing rule that a score delta between two runs is
  only reported as real ("signal") if it clears a statistical significance
  test; otherwise it's labeled `inside_noise_floor` and Crucible refuses to
  claim one model beat another. This is the concrete mechanism behind the
  repo's stated principle ("refuse to report a delta it cannot defend").
  Represented as `DeltaVerdict::{Signal, InsideNoiseFloor}` in
  `crucible-core/src/measure/paired.rs`.

- **DeltaVerdict** — the two-value enum (`Signal` / `InsideNoiseFloor`) that
  carries the noise-floor decision as persisted data, so a stored comparison
  doesn't need to re-derive its own verdict from a raw p-value later. Defined
  in `crucible-core/src/measure/paired.rs` (`enum DeltaVerdict`).

- **McNemar / paired comparison** — the statistical test `crucible runs
  compare` uses when two runs share task ids: it counts how many tasks flipped
  from pass to fail (or vice versa) between the two runs and tests whether
  that flip count is more than noise. This is what actually produces the
  `DeltaVerdict` above. Implemented in `crucible-core/src/measure/paired.rs`
  (`PairedComparison::mcnemar`) and wired into run comparisons in
  `crucible/src/run_store.rs` (`fn paired_mcnemar`).

- **pass^k** — a stricter pass-rate metric for benchmarks that repeat the same
  task across multiple trials (`k` trials per task): a task only counts as a
  pass if *every* trial for that task passed, then the fraction of
  all-trials-passed tasks gets its own Wilson interval. It only computes when
  every selected task shares the same trial count (≥ 2); otherwise it's
  omitted rather than silently approximated. Computed in
  `crucible/src/spec_run.rs` (`fn compute_pass_k`) and indexed for comparison
  in `crucible/src/run_store.rs` (`fn merge_pass_k_task_rows`).

- **Judge-gaming canary** — a planted, obviously-bad candidate answer that an
  `agentic_judge` run always includes alongside real candidates. If the live
  model judge rubber-stamps that known-bad candidate as good, the runner
  treats the judge as untrustworthy for this run and hard-refuses — no
  evidence gets persisted — rather than silently recording a judge score it
  has reason to believe is broken. Implemented in `crucible/src/spec_run.rs`
  (search `judge-gaming guard tripped`) and described in
  `crucible-core/src/spec.rs` near the `Agentic` grader kind.

- **Calibration record** — the measured agreement between a live model judge
  and human labels on a set of labeled calibration tasks: raw percent
  agreement, Cohen's κ (chance-corrected agreement), and the judge-vs-human
  confusion matrix. This is the evidence that licenses trusting a judge's
  score at all, separate from the judge-gaming canary above (which catches a
  judge being gamed on *this* run, not whether the judge is calibrated in
  general). Defined in `crucible-core/src/calibration.rs`
  (`struct CalibrationRecord`).

- **Harbor** — the Threshold/Daedalus on-disk scorer format Crucible imports
  from and exports to: `solution/findings.json` (the answer-key oracle) and
  `tests/expected.json` (the scorer key), keyed by arena and task id (e.g.
  `pr-review-v0` / `py-file-cache`). Crucible does not invent its own export
  schema — it targets Harbor because that's the format the consuming
  Threshold/Daedalus optimization loop already reads. See
  `crucible-core/src/export.rs` and `crucible-core/src/spec.rs` (the
  `daedalus_trials` corpus source and `tests/expected.json` scorer key).

- **Findings journal** — the durable, append-only record of comparisons that
  actually cleared the noise floor (`DeltaVerdict::Signal`) — a place to look
  for "what did we learn," separate from the raw run ledger described below,
  which stores every run whether or not its comparisons were conclusive.
  Defined in `crucible/src/findings_journal.rs` (`struct FindingsJournal`).

- **Run ledger** — the gitignored SQLite database (default
  `runs/local/crucible-runs.sqlite`) every `crucible run` invocation writes
  rows into, queryable via `crucible runs list/show/compare` from the CLI or
  MCP. It's the single source `crucible serve`'s UI reads from too — the
  browser workbench is a readback of the same ledger, not a separate scoring
  system. See `crucible/src/run_store.rs` (`DEFAULT_DB_PATH`).
