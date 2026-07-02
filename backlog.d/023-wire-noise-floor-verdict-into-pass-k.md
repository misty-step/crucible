# Wire the noise-floor verdict into pass^k reporting

Priority: P1 · Status: ready · Estimate: M

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
