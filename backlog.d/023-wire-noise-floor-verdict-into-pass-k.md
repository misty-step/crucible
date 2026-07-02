# Wire the noise-floor verdict into pass^k reporting

Priority: P1 · Status: blocked (needs design decision) · Estimate: M

## Goal

`crucible-core::measure::paired::PairedComparison`/`DeltaVerdict` (the
McNemar-based "refuse a delta you cannot defend" kernel — already used by
`run_store::compare_configs` and the dashboard leaderboard) exists and is
proven, but `backlog.d/015-first-real-cerberus-review-benchmark.md`'s own
Notes name it as "not wired to this benchmark yet": the pass^k score
(`compute_pass_k`, `crucible/src/spec_run.rs:889-933`) reports a single
Wilson-interval point estimate per run, with no paired significance
verdict when comparing pass^k across two configs/runs of the same
benchmark.

## Oracle

- [ ] `crucible runs compare` (or a benchmark-specific path if the shared
  `compare_configs` doesn't apply cleanly to pass^k's per-task boolean
  outcome) computes a `PairedComparison::mcnemar` + `DeltaVerdict` over two
  runs' pass^k task-level pass/fail outcomes when both runs share the same
  task set — mirroring exactly the pattern `run_store.rs`'s
  `compares_latest_runs_by_model_as_a_paired_mcnemar_delta` test already
  proves for prompt-benchmark task rows.
- [ ] The verdict (`Signal` / `InsideNoiseFloor`) is surfaced in the CLI/JSON
  output alongside the existing pass^k point + Wilson CI, not silently
  computed and dropped.
- [ ] A new test runs two `cerberus-review-quality-v0`-shaped fixture
  invocations (or a smaller fixture spec with the same runner kind) with
  known discordant pass/fail pairs and asserts the verdict matches a
  hand-computed McNemar result.
- [ ] `cargo test --all` passes; no change to the existing
  `compute_pass_k`/Wilson reporting path for a single run (no paired
  baseline available).

## Notes

Live-code-verified 2026-07-01: `crucible-core/src/measure/paired.rs` fully
implements `PairedComparison::mcnemar`/`DeltaVerdict` and is already wired
into `run_store::compare_configs` (`crucible/src/run_store.rs:486-585`,
tested at lines 1531-1591) for prompt-benchmark task rows sharing a
`task_id`. `backlog.d/015...md`'s Notes explicitly list this as open: "a real
noise-floor verdict on the pass^k delta (this epic's `PairedComparison`/
`DeltaVerdict` kernel already exists in `crucible-core`, just not wired to
this benchmark yet)." This is deliberately scoped as *wiring*, not new
statistics — the exact same pattern PR #62 already applied to prompt-
benchmark comparisons tonight.

**Why:** epic-named remaining work with the hard statistical/design part
already solved and proven elsewhere in the same codebase — matches
OVERNIGHT.md's "calibration/adjudication polish" bucket.

**Skipped 2026-07-02 (overnight) — the ticket's own premise does not hold on
live-tree verification.** The claim "deliberately scoped as *wiring*, not new
statistics — the exact same pattern PR #62 already applied to prompt-benchmark
comparisons" is false for pass^k specifically: `run_store.rs`'s paired-McNemar
path in `compare_configs` pairs on `StoredPromptTask` rows fetched from the
`prompt_task_results` SQL table via `query_prompt_tasks`. Those rows are
populated only when a run's evidence carries schema
`crucible.prompt_run_evidence.v1` or `crucible.agentic_judge_evidence.v1` (see
`extract_metadata`'s dispatch, `run_store.rs`). `key_recall`'s evidence —
`crucible.spec_run_evidence.v1`, the schema `cerberus-review-quality-v0` and
every pass^k-bearing run uses — routes to `merge_spec_metadata` instead, which
does **not** populate `metadata.prompt_tasks` at all (confirmed by reading the
function: it sets only `runner_kind`/`spec_path`/`config_id`). So today, zero
`key_recall` runs have ANY per-task rows in the SQL ledger to pair on —
`compare_configs`'s existing logic has nothing to read for a pass^k
comparison, full stop.

Making this real requires first deciding how `key_recall`'s per-task
pass/fail rows (the ones `compute_pass_k` already computes in-process from
`TaskResult`) get persisted into the run store at all: reuse
`prompt_task_results`'s shape under a generalized name, add a parallel table,
or store them differently given `key_recall` tasks aren't 1:1 with "prompt
tasks" the way `prompt_benchmark`/`agentic_judge` tasks are. That's a real
schema/persistence-pipeline decision, not "wiring" an already-proven pattern —
exactly the kind of design call this overnight lane defers rather than
deciding unilaterally. Falling back to a smaller, unambiguous ticket instead.
Un-skip this once someone decides where `key_recall` per-task rows live in
the ledger.
