# Crucible repo contracts

- North star: read `VISION.md` before changing product scope, eval semantics,
  grader/judgment boundaries, runner boundaries, UI direction, or the
  Daedalus/Harness Kit relationship.
- Current state: the author-and-run engine is real. Three runner kinds
  (`key_recall`, `prompt_benchmark`, `agentic_judge`) execute declared
  `EvalSpec`s through `crucible run`/MCP `crucible_run`, including live BYOK
  OpenRouter model calls; every run persists to a SQLite ledger
  (`runs/local/crucible-runs.sqlite`) queryable via `crucible runs
  list/show/compare/history/pivot` (CLI + MCP) — config identity now carries
  explicit `harness`/`tool_allowlist` fields (`backlog.d/027-*`), `history`
  is one config's score trend over time, `pivot` is one benchmark's latest
  run per model narrowable to one harness. **Config-identity axes**
  (`crucible-973`, the complete set, documented once here rather than only
  derivable from `run_store.rs` internals): `provider`, `model`, `temp`,
  `max` output units, the system/judge prompt hash, a `scoring` identity
  (below-harness-name grading: `rubric_hash` per task — `expectation_kind`
  + value for `prompt_benchmark`, rubric text for `agentic_judge` —
  aggregated so a corpus that changes its grading definitions gets a
  genuinely distinct `config_id`, never silently sharing history with the
  old grader), and optionally `harness`/`tool_allowlist`. Response-model
  drift — a provider silently changing the model behind a requested slug
  (arXiv:2407.12220's QRPs catalog documents harness hacking *below* the
  harness-name level) — is a related but separate axis: every run's uniform-
  or-empty `response_model` is aggregated and persisted (`run_records
  .response_model`), and `runs history`/`compare` warn (never silently drop)
  when runs sharing one requested model slug recorded differing response
  models. Every paired comparison
  (`compare`'s `paired`/`class_breakdowns[].paired`) also carries a
  `resolution` (`crucible-950`): Kotawala's resolution ratio `q = n/N*` and
  minimum detectable effect, from the correct paired-Bernoulli variance
  formula (`crucible_core::required_n_paired`/`minimum_detectable_effect_paired`,
  not the unpaired Cohen's-h-times-`(1-rho)` shortcut), with a `diagnosis`
  that distinguishes an `InsideNoiseFloor` verdict's "no_effect" (adequately
  powered, found nothing) from "underpowered" (cannot rule out an effect of
  that size) from "no_discordance" (perfect agreement). `EvalSpec.min_effect_of_interest`
  is the prospective counterpart: `crucible validate` warns (conservative
  one-sample proxy, no paired data exists pre-run) when the declared task
  count cannot resolve it at `(alpha=0.05, power=0.8)`. `crucible validate`/MCP `crucible_validate`
  checks a spec's `{valid, runnable, errors, warnings}` before it runs, and the
  runner refuses (not silently ignores) an unsupported `aggregation`,
  `uncertainty.method`/`confidence`, or a missing grader of the kind the
  runner's family actually executes. The agentic judge tier
  (`backlog.d/012-*`) is real: a live judge call, a `CalibrationRecord`
  measuring judge-vs-deterministic agreement on labeled calibration tasks, and
  a judge-gaming canary that hard-refuses a run (no evidence persisted) if the
  judge rubber-stamps a known-bad candidate. The judge protocol is
  reasoning-first (`crucible-969`): the judge is instructed to reason before
  the verdict, `parse_judge_verdict` is tail-anchored (only the final line's
  `VERDICT:` tag is read, so a pre-2026-07-06 verdict-first response is
  rejected, not silently accepted). `AgenticJudgeTask.reference` injects an
  optional known-perfect exemplar labeled as such (never as the candidate);
  `AgenticJudgeConfig.format_sensitivity_check` (opt-in) re-probes every
  decisive calibration item with a cosmetically reordered prompt and records
  the flip rate as `CalibrationRecord.format_sensitivity_flip_rate`/`_n`.
  `CalibrationRecord` v2 (`crucible-970`) adds fail-class precision/recall
  (`fail_class_precision`/`_recall`, the minority-class metric a bare Cohen's
  κ hides), a `task_family` axis folded into `judge_licence_key` (bumped
  `v1`→`v2`) so a licence earned on one task family cannot silently cover
  another, an opt-in cross-run drift check (`probe_drift`,
  `AgenticJudgeConfig.previous_evidence_path`) distinct from the within-run
  format-sensitivity self-check, and `expected_verdicts_from_labels` — a
  documented Keep/Nit→pass, Wrong/Noise→fail mapping so blind
  `crucible.label.v1` judgments can source calibration ground truth alongside
  a spec's declared `expected_pass`. The calibration gate this record measures
  is structural, not a note string (`crucible-971`): every persisted run
  carries `run_records.trusted` (`true` unless it is a locked/unmeasured
  `agentic_judge` run), and `runs compare` refuses — `comparison_kind:
  "untrusted_run_refused"`, `paired`/`resolution` left `None` — any
  comparison naming an untrusted run, which makes a Signal
  `crucible.finding.v1` from a locked judge structurally impossible (the
  findings journal derives every finding from `paired`). `runs compare` also
  labels every comparison's attribution (`crucible-974`): derived from the
  real identity diff between the two resolved runs (never assumed from the
  query strings), `"model_delta"`/`"harness_delta"` when exactly that one
  axis differs, `"config_delta"` otherwise — SWE-bench-Lite swung
  2.7%->28.3% for the same model on harness alone, so a delta spanning two
  axes at once is unattributable by construction. `--strict` (CLI) /
  `strict` (MCP) refuses a `config_delta` comparison outright instead of
  rendering it with a caveat; `FindingRecord.comparison_type` carries the
  same label downstream. Env-backed (`harbor_task`) comparisons additionally
  get a `resource_envelope_caveat` when their declared `ResourceEnvelope`s
  (cpu/mem/headroom, `HarborRunConfig.resource_envelope`) mismatch, or when
  neither side declared one and the delta is small enough that Anthropic's
  Feb 2026 infrastructure-noise finding (container CPU/RAM headroom-vs-limit
  alone produced a 6pp swing on Terminal-Bench 2.0) could plausibly explain
  it. `AgenticJudgeTask.rubric` (`crucible_core::Rubric`) is either a single
  holistic string (back-compat) or a named `Criteria` list, each judged by
  its own isolated call (`crucible-952`) — RubricEval (arXiv:2603.25133)
  found isolated per-criterion calls beat one call over the whole rubric by
  7-12 balanced-accuracy points and roughly halve inter-judge variance. Task
  verdict aggregates per-criterion verdicts by `criteria_aggregation`
  (`AllMustPass` today: any `Fail` decisively fails the task, an `Unknown`
  without a `Fail` is `Unknown`); trace steps and evidence carry per-criterion
  labels (`"{task_id}:{criterion_name}"`); the format-sensitivity self-check
  redispatches and reaggregates every criterion when reprobing. `judge_stats`
  reports `multi_criterion_call_overhead`/`_cost_overhead_usd` — the extra
  calls (and their dollar cost) versus one call per task — surfaced in the
  run's notes. Live evidence (`anthropic/claude-haiku-4.5`, 8-task
  code-review-comment-quality calibration set): per-criterion agreement
  (1.00) >= checklist-mode agreement (1.00) on this family, with per-criterion
  dispatch additionally isolating exactly which criterion failed per task
  (e.g. a vague-fix task's trace shows `identifies_real_issue: pass`,
  `actionable_fix: fail`) — attribution a single holistic verdict cannot give
  regardless of judge quality. The agentic-judge runner also
  persists a `Trace` (`crucible-core::trace`, `backlog.d/030-*`) — an ordered
  judge_call/verdict_parsed/calibration_check step sequence pointed to from
  `run_records.trace_path` and surfaced via `runs list/show`/MCP the same way
  `evidence_path`/`spec_path` are, so a failed or UNKNOWN-verdict run is
  inspectable without re-running it; `prompt_benchmark`/`key_recall` are not
  yet wired to emit one. The adjudication panel has a real
  writeback loop (`adjudication-panel --serve`, `backlog.d/005-*`) — a small
  local HTTP server that persists Keep/Nit/Wrong/Noise taps as
  `crucible.label.v1` labels through the same `apply_label` path
  `adjudicate --apply` uses. `crucible author` (crucible-942) assembles a
  valid `EvalSpec` from flags or a guided `--interactive` prompt flow for
  `key_recall`/`prompt_benchmark`, running the same validation `crucible
  validate` performs before saving — the brainstorm/design/define lifecycle
  stage no longer requires hand-writing JSON. `crucible import <adapter>
  <source>` (crucible-026) is the other direction: it projects an
  externally-authored eval/benchmark definition onto a valid `EvalSpec`
  through the same validate-then-save gate — the first adapter, `promptfoo`,
  projects a Promptfoo-style YAML config onto the `prompt_benchmark` runner,
  reporting (never silently dropping) any test case it cannot map cleanly.
  See `SKILL.md` for the exact commands. Do not invent a broad platform stack
  ahead of real usage; open work lives in `backlog.d/` (deterministic grader
  dispatch beyond the required-kind check, judge-calibration model-family
  separation, baseline comparison wiring, the phone-adjudication epic's
  remaining UI polish, `agentic_judge` authoring in `crucible author`, an MCP
  `crucible_import` tool mirroring `crucible_author`, and a second import
  adapter — e.g. a Threshold/Daedalus arena format — once the `key_recall`
  runner has a way to execute fresh candidate output rather than only
  grading already-produced trials).
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
