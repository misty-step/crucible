# Crucible repo contracts

- North star: read `VISION.md` before changing product scope, eval semantics,
  grader/judgment boundaries, runner boundaries, UI direction, or the
  Daedalus/Harness Kit relationship.
- Current state: the author-and-run engine is real. Three runner kinds
  (`key_recall`, `prompt_benchmark`, `agentic_judge`) execute declared
  `EvalSpec`s through `crucible run`/MCP `crucible_run`, including live BYOK
  OpenRouter model calls; every run persists to a SQLite ledger
  (`runs/local/crucible-runs.sqlite`) queryable via `crucible runs
  list/show/compare` (CLI + MCP). `crucible validate`/MCP `crucible_validate`
  checks a spec's `{valid, runnable, errors, warnings}` before it runs, and the
  runner refuses (not silently ignores) an unsupported `aggregation`,
  `uncertainty.method`/`confidence`, or a missing grader of the kind the
  runner's family actually executes. The agentic judge tier
  (`backlog.d/012-*`) is real: a live judge call, a `CalibrationRecord`
  measuring judge-vs-deterministic agreement on labeled calibration tasks, and
  a judge-gaming canary that hard-refuses a run (no evidence persisted) if the
  judge rubber-stamps a known-bad candidate. The adjudication panel has a real
  writeback loop (`adjudication-panel --serve`, `backlog.d/005-*`) — a small
  local HTTP server that persists Keep/Nit/Wrong/Noise taps as
  `crucible.label.v1` labels through the same `apply_label` path
  `adjudicate --apply` uses. See `SKILL.md` for the exact commands. Do not
  invent a broad platform stack ahead of real usage; open work lives in
  `backlog.d/` (deterministic grader dispatch beyond the required-kind check,
  judge-calibration model-family separation, baseline comparison wiring, the
  phone-adjudication epic's remaining UI polish).
- Boundary (rechartered 2026-06-29, refreshed 2026-07-01): Crucible owns the
  eval/benchmark as a durable artifact — definition, design, implementation,
  selected execution, calibration, run records, judging, reporting, and export.
  Threshold/Daedalus runs Karpathy-style optimization loops that consume
  Crucible's trusted evals and run records. Eval-authoring machinery migrates
  from Daedalus into Crucible over time (`backlog.d/007-*`).
- Do not reinvent eval infrastructure. Borrow commodity execution and ordinary
  grading where they plug in — the existing Daedalus arenas/corpus/Harbor format
  and Cerberus for the code-review wedge; frameworks like Promptfoo or Inspect AI
  for future families where they fit. Crucible owns the eval artifact, selected
  run execution, the calibration/trust layer, the human-judgment surface, run
  records, and the export contract.
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
- Verification skill: `SKILL.md` is the cold-agent command contract — the
  three built-in eval receipts, declared-spec runs across all three runner
  kinds, `crucible validate`, the SQLite run ledger queries, the headless
  grade/adjudicate/export loop, the adjudication panel (static and
  `--serve` writeback), and the dashboard.

## Gate

The repo gate is `scripts/check.sh` (also `make check`):

```sh
./scripts/check.sh
```

It runs, across the whole workspace and fails on the first error:

```sh
scripts/leak-scan.sh          # credential-leak scan (security floor)
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo build --all
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

Run it before pushing and wire it into CI unchanged. Do not weaken it to get
green (no `--no-verify`, no removed `-D warnings`, no skipped tests). As the
eval surface lands, extend the gate with Harbor export validation and keep this
section current. See `backlog.d/006-agent-readiness-machine-surface.md`.

### Content & secret policy

Eval runs invoke models with real API keys and store their outputs over real PR
diffs that can embed proprietary code. Two standing rules, enforced differently:

- **No credentials in the tree** — enforced mechanically by the gate's first
  step, `scripts/leak-scan.sh`: a self-contained high-signal grep floor over
  tracked files, plus gitleaks' broad ruleset when it is on PATH. It matches
  *credential shapes* — private keys (incl. PGP), AWS keys, bearer tokens,
  OpenAI/Anthropic/Slack/GitHub tokens, Stripe/Google API keys, JWTs,
  URL-embedded credentials, and `api|secret|token=<value>` assignments — and
  fails the gate on a hit. If a matched credential was ever real, rotate it. The
  scan detects credential *shapes*, not arbitrary proprietary text; confining
  raw content is the next rule, which is policy, not pattern-matching.
- **Raw model outputs and raw diffs live only under allowlisted fixture dirs**
  (`crucible*/tests/fixtures/`) — enforced by review, not the scanner. There
  they are committed deliberately as test inputs and must carry no live secret.
  Eval run records — which embed real diffs and API-keyed transcripts — are
  written under `runs/` (gitignored in full), never committed raw; redact or
  allowlist before anything is published.
