# Crucible repo contracts

- North star: read `VISION.md` before changing product scope, eval semantics,
  grader/judgment boundaries, runner boundaries, UI direction, or the
  Daedalus/Harness Kit relationship.
- Current state: docs-first seed repo; no application code yet. The first
  implementation is the code-review eval wedge (`backlog.d/002-*`). Do not invent
  a broad platform stack ahead of it.
- Boundary (rechartered 2026-06-29): Crucible owns the eval/benchmark as a
  durable artifact — definition, design, implementation, calibration, run
  records, judging, reporting, and export. Daedalus runs Karpathy-style
  optimization loops that consume Crucible's trusted evals. Eval-authoring
  machinery migrates from Daedalus into Crucible over time (`backlog.d/007-*`).
- Do not reinvent eval infrastructure. Borrow execution and ordinary grading
  where they plug in — the existing Daedalus arenas/corpus/Harbor format and
  Cerberus for the code-review wedge; frameworks like Promptfoo or Inspect AI for
  future families where they fit. Crucible owns the eval artifact, the
  calibration/trust layer, the human-judgment surface, and the export contract.
- Judgment is a per-eval decision across deterministic, agentic, and human
  layers; most real evals are hybrid and a good portion need some human judgment.
  Calibrate agentic/model judges against human labels before trusting them.
- The one principle: Crucible refuses to report a delta it cannot defend — every
  rate carries an interval, every judge a calibration, every comparison a
  noise-floor check.
- Rust by default for the durable Crucible-owned core (eval object, calibration,
  uncertainty, storage, export, validation). A TypeScript/React web layer is
  acceptable when the human-judgment UI is the work; keep that boundary explicit.
  Execution and commodity grading are borrowed, not rebuilt.
- Exports align to the consumer's contract (the Daedalus Harbor task-directory
  format for code-review), not a Crucible-invented schema.
- Backlog: active work lives in `backlog.d/NNN-*.md`; closed work moves to
  `backlog.d/_done/`.

## Gate

The repo gate is `scripts/check.sh` (also `make check`):

```sh
./scripts/check.sh
```

It runs, across the whole workspace and fails on the first error:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo build --all
```

Run it before pushing and wire it into CI unchanged. Do not weaken it to get
green (no `--no-verify`, no removed `-D warnings`, no skipped tests). As the
eval surface lands, extend the gate with Harbor export validation and a
secret/content-leak scan and keep this section current. See
`backlog.d/006-agent-readiness-machine-surface.md`.
