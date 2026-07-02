# Add a Trace layer: capture and inspect what actually happened in a run

Priority: P2 · Status: pending · Estimate: M

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

- [ ] A `Trace` type exists in `crucible-core` recording, at minimum: ordered
  steps, per-step kind (model call / tool call / retrieval / other), inputs,
  outputs, timestamps, and token/cost where applicable — general enough to
  cover prompt-benchmark, agentic-judge, and a future agent-trajectory runner
  without a bespoke shape per runner kind.
  Structural cross-reference (naming and layering only — not
  binding on Crucible's implementation): `ai-evals-benchmarks-report.md` §8's
  `Trace` domain object.
- [ ] At least one runner (agentic-judge is the natural first candidate,
  since it already makes a real model call) persists a `Trace` alongside its
  existing run record, queryable via `runs show`/MCP.
- [ ] A failed or `unknown`-verdict run (see `029`) can be inspected via its
  trace without re-running the candidate.
- [ ] Traces are stored under the same artifact-pointer discipline as
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
