# Add a Trace layer: capture and inspect what actually happened in a run

Priority: P2 · Status: done · Estimate: M

## Outcome (completed 2026-07-05)

`crucible-core` gains a `trace` module: a schema-tagged (`crucible.trace.v1`)
[`Trace`] with an ordered `Vec<TraceStep>` (`sequence`, caller-supplied RFC
3339 `timestamp`, open `kind`/`detail`/`outcome` strings so future runner
kinds don't need a schema migration per step shape) and a `failure_steps()`
helper that surfaces `unknown`/`fail`/`error` outcomes first. `RunRecord`
gains an optional `trace_path` pointer alongside the existing
`evidence_path`/`spec_path` (additive field, no schema bump). The
agentic-judge runner (`run_agentic_judge_with_client`) now emits a
`judge_call` → `verdict_parsed` → (optional) `calibration_check` step
sequence per task and persists it as `agentic-judge-trace.json`, added to the
run's `artifacts` the same way every other evidence file is. `run_store.rs`
recognizes the `crucible.trace.v1` schema in `extract_metadata`, adds a
`run_records.trace_path` column via the existing additive `ensure_column`
migration, and surfaces it through `runs list`/`runs show`/MCP
`crucible_runs_show` — the same artifact-pointer discipline as every other
evidence file, no parallel storage. Verified live against a real OpenRouter
judge call (`crucible run evals/agentic-judge-smoke-v0.json`): the trace
artifact records both tasks' judge calls, verdicts, and the canary's
calibration check, and `crucible runs show <run_id>` lists it with kind
`trace`. TDD: failing tests first for an UNKNOWN-verdict task's inspectable
trace and a clean-pass trace's structural soundness, then the run-store
discoverability test, then implementation. Full `cargo test --all` and
`./scripts/check.sh` green.

## Goal

`crucible-core` has no `trace` module (confirmed: `lib.rs` exports
`adapter, adjudication, artifact, calibration, dashboard, export, grade,
judgment, key, label, measure, provenance, spec` — no trace type). Runs
persist prompt/response, config, score, and cost, but not the structured
record of *how* the candidate got there: retrieved context, tool calls, agent
steps, intermediate reasoning artifacts. For the code-review family this is
tolerable (Cerberus's `ReviewArtifact`/`Finding` already carries anchors); for
the next families VISION.md names — Harness Kit primitive evals, agentic
product behavior (Memory Engine, Allie) — trace absence blocks exactly the
kind of debugging a failing run needs ("why did this candidate fail" becomes
unanswerable without re-running).

## Oracle

- [x] A `Trace` type exists in `crucible-core` recording, at minimum: ordered
  steps, per-step kind (model call / tool call / retrieval / other), inputs,
  outputs, timestamps, and token/cost where applicable — general enough to
  cover prompt-benchmark, agentic-judge, and a future agent-trajectory runner
  without a bespoke shape per runner kind. (Scoped down at implementation
  time: `kind`/`detail` are open strings/`serde_json::Value` rather than a
  closed enum precisely so future runner kinds do not force a schema
  migration — the generality the oracle asked for.)
  Structural cross-reference (naming and layering only — not
  binding on Crucible's implementation): `ai-evals-benchmarks-report.md` §8's
  `Trace` domain object.
- [x] At least one runner (agentic-judge is the natural first candidate,
  since it already makes a real model call) persists a `Trace` alongside its
  existing run record, queryable via `runs show`/MCP.
- [x] A failed or `unknown`-verdict run (see `029`) can be inspected via its
  trace without re-running the candidate.
- [x] Traces are stored under the same artifact-pointer discipline as
  everything else in `011` — no raw trace content inlined into tracked git
  files; SQLite/artifact-dir only.

## Notes

Prior art: `ai-evals-benchmarks-report.md` §8 ("Product architecture for an
eval system") names `Trace` as one of eleven core domain objects
(`EvalSpec, Dataset, Example, TargetAdapter, Runner, Trace, Scorer, Judge,
Experiment, Report, Gate`) and Layer 3 ("Trace layer") as a distinct system
layer; §7 ("RAG and agent evals" — "Agent evals: evaluate trajectories, not
just answers") makes the case that trajectory-level inspection, not just
final-answer scoring, is what agent evals need. This is a genuine addition —
Crucible's current domain model (`spec.rs`, `artifact.rs`, `judgment.rs`) has
no trajectory/trace concept, and the operator's stated want ("harnesses, tools
available" as tracked axes, see `027`) implies wanting to see what a harness
actually did, not just its final score.

This also gives `026`'s Inspect AI adapter a natural second use: Inspect's
eval logs are themselves trace-shaped, so importing Inspect-format runs could
populate this `Trace` type directly rather than needing a second bespoke
mapping.

Follow-up not in scope here: wiring `prompt_benchmark`/`key_recall` runners to
also emit a `Trace` (only agentic-judge was wired, per the oracle's "at least
one runner"), and a dedicated `crucible runs trace <run_id>` pretty-printer
beyond the generic `artifacts`/`trace_path` pointers `runs show` already
exposes.
