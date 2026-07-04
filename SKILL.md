---
name: crucible
description: Run and extend Crucible evals. Use when an agent needs to author, run, grade, adjudicate, export, or report Crucible code-review evals from this repo.
---

# Crucible Verification Skill

Crucible owns eval artifacts, judgment queues, uncertainty, and exports. The
first family is agentic code-review quality over Cerberus findings and
Threshold/Harbor scorer keys.

## Start Here

Read `VISION.md` and `AGENTS.md` before changing eval semantics, grader
boundaries, runner boundaries, exports, or UI. Raw model outputs and raw diffs
must stay under `runs/` unless deliberately committed as sanitized fixtures under
`crucible*/tests/fixtures/`.

## Operator Onboarding: Crucible In 10 Minutes

When the operator asks "how do I actually define and run a benchmark?", start
with [`docs/operator-walkthrough.md`](docs/operator-walkthrough.md). It walks a
cold reader through the smallest useful Crucible loop:

```sh
cargo run -p crucible -- validate evals/operator-micro-benchmark-v0.json --json

cargo run -p crucible -- run evals/operator-micro-benchmark-v0.json \
  --models deepseek/deepseek-v4-flash,z-ai/glm-5.2 \
  --out runs/local/crucible-101/final \
  --db runs/local/crucible-101/final.sqlite \
  --json

cargo run -p crucible -- runs compare \
  --benchmark operator-micro-benchmark-v0 \
  --left deepseek/deepseek-v4-flash \
  --right z-ai/glm-5.2 \
  --db runs/local/crucible-101/final.sqlite \
  --json

cargo run -p crucible -- serve \
  --db runs/local/crucible-101/final.sqlite \
  --specs evals \
  --port 4174
```

The expected operator-level readout is: DeepSeek and GLM both produce stored
run receipts, task-level pass/fail verdicts are visible in the UI's Receipts
view, and the paired comparison reports any tiny-sample delta as inside the
noise floor unless it is actually defensible.

## Author A New Eval Spec (No Hand-Written JSON)

`crucible author` assembles a valid `EvalSpec` from flags — a scriptable,
cold-agent-friendly path — or a guided `--interactive` stdin/stdout prompt
flow (plain `read_line`, no TUI dependency). Either way it runs the assembled
spec through the exact same validation `crucible validate` performs and
prints `{valid, runnable, errors, warnings}` before saving; an invalid
assembly is refused and leaves no file behind.

```sh
cargo run -p crucible -- author \
  --id my-eval-v0 \
  --task-family prompt-smoke \
  --runner-kind prompt_benchmark \
  --prompt-model openrouter/auto \
  --prompt-system-prompt "Answer exactly." \
  --prompt-task-id marker-echo \
  --prompt-task-prompt "Reply with crucible-smoke" \
  --prompt-expectation-kind contains \
  --prompt-expectation-value crucible-smoke \
  --out evals/my-eval-v0.json \
  --json
```

Or walk through the same fields interactively:

```sh
cargo run -p crucible -- author --interactive --out evals/my-eval-v0.json
```

Covers `key_recall` (over a Daedalus `trials.jsonl` corpus, `--key-recall-*`
flags) and `prompt_benchmark` (`--prompt-*` flags, one authored task per
invocation — hand-edit the `tasks` array or re-run `author` for additional
tasks). `agentic_judge` authoring is a documented follow-up (backlog.d/):
its judge-gaming canary and calibration-probe shape need a richer prompt
flow than this pass's flag/stdin surface covers well. When no `--grader` is
named, one canonical grader of the chosen runner's required kind is added
automatically so the spec is runnable, not merely definition-only; an
explicit grader mix that still lacks the required kind is left as declared
so the save-gate validation reports it, rather than the CLI silently
rewriting it. The result is a real `evals/*.json` file — usable by
`crucible run`/`validate`/`serve` exactly like a hand-written one.

## Validate A Spec Before Running

Check a declared spec is an executable contract — no sibling checkout, no
trials file, no `OPENROUTER_API_KEY` required:

```sh
cargo run -p crucible -- validate evals/pr-review-key-recall-v0.json --json
```

Returns `{valid, runnable, errors, warnings}`. `errors` name a field the
runner will refuse to run over (wrong `aggregation`/`uncertainty.method`, a
declared `uncertainty.confidence` other than `0.95` — the only interval the
runner computes — or a missing grader of the kind the runner's family
actually executes). `warnings` name fields that are honestly not yet wired
(`baselines`) or informational (a `daedalus_trials` corpus path that escapes
the spec's own directory, so it only runs against a specific sibling
checkout). Exits `0` whether or not the spec is valid — the verdict is in the
body, same as `grade`/`adjudicate`; exit `1` is a genuine load error (unknown
`schema_version`, malformed JSON).

## Run Declared And Built-In Evals

Run the first declared benchmark spec:

```sh
cargo run -p crucible -- run evals/pr-review-key-recall-v0.json --json
```

The spec writes `runs/local/pr-review-key-recall-v0/run-report.json` by default.
It measures Threshold `pr-review-v0` key recall for the selected
`probe-oneshot` candidate over the frozen six-task corpus. The sibling checkout
and scorer binary still use the `daedalus` name until that repo is physically
renamed.

Run the first Crucible-owned prompt benchmark through OpenRouter:

```sh
OPENROUTER_API_KEY=... \
cargo run -p crucible -- run evals/prompt-smoke-v0.json \
  --out runs/local/prompt-smoke \
  --json
```

This is the first author-and-run engine slice: Crucible owns the authored prompt
benchmark, makes the live model call, grades the text with a deterministic
rubric, writes `prompt-run.json` evidence under `runs/`, and persists the run
plus prompt task rows into the SQLite ledger at
`runs/local/crucible-runs.sqlite`.

Run the deterministic tracer benchmark across real OpenRouter models with one
committed spec and no judge calls:

```sh
cargo run -p crucible -- run evals/tracer-exact-v1.json \
  --models deepseek/deepseek-v4-flash,z-ai/glm-5.2,moonshotai/kimi-k2.7-code \
  --out runs/local/tracer-exact-v1 \
  --json

cargo run -p crucible -- runs compare \
  --benchmark tracer-exact-v1 \
  --left deepseek/deepseek-v4-flash \
  --right z-ai/glm-5.2
```

Run the class-stratified discriminator battery (`tracer-exact-v2`): 60
deterministic tasks, 15 each for `code_output`, `long_context_extraction`,
`format_adherence`, and `arithmetic_logic`. It uses no judge models; code
outputs are graded by executing committed Python unit tests in a temporary
directory, format tasks by strict JSON parsing/equality, and the other classes
by exact match.

```sh
cargo run -p crucible -- run evals/tracer-exact-v2.json \
  --models deepseek/deepseek-v4-flash,z-ai/glm-5.2,moonshotai/kimi-k2.7-code,google/gemini-2.5-flash-lite \
  --out runs/local/tracer-exact-v2/full \
  --db runs/local/tracer-exact-v2/crucible-runs.sqlite \
  --json

cargo run -p crucible -- runs compare \
  --benchmark tracer-exact-v2 \
  --left deepseek/deepseek-v4-flash \
  --right google/gemini-2.5-flash-lite \
  --db runs/local/tracer-exact-v2/crucible-runs.sqlite \
  --json
```

`--model` is only a run-time override for declared `prompt_benchmark` specs; it
keeps the authored eval stable while comparing selected model slugs.
`--models` is the fan-out form for the same surface: comma-separated model slugs
run one at a time, each under its own output child directory and each persisted
as a normal run row with its own config/model identity.
For prompt benchmarks that declare per-task `class`, `runs compare` emits
`class_breakdowns` rows with per-class pass counts, deltas, paired task counts,
and McNemar noise-floor verdicts in addition to the overall comparison.

Use an isolated ledger for tests or one-off proof:

```sh
cargo run -p crucible -- run evals/prompt-smoke-v0.json \
  --out runs/local/prompt-smoke \
  --db runs/local/crucible-runs.sqlite \
  --json
```

Run the agentic judge (`GraderKind::Agentic` made real, backlog 012): a live
BYOK judge model scores a candidate against a rubric, with a judge-gaming
canary — a deliberately bad candidate the judge must reject:

```sh
OPENROUTER_API_KEY=... \
cargo run -p crucible -- run evals/agentic-judge-smoke-v0.json \
  --out runs/local/agentic-judge-smoke \
  --json
```

If the judge rubber-stamps the canary (agrees it passes when the spec says it
must not), the run refuses outright — no evidence is written, not even
`run-report.json`. Every task carrying a known `expected_pass` (the canary is
one example) also feeds a `CalibrationRecord`: raw agreement, Cohen's κ, and
an `unlocked` flag (agreement ≥ 0.8) recorded in the evidence JSON and spelled
out in the run's notes as "Calibration UNLOCKED"/"LOCKED".

Run the first real Cerberus review-quality benchmark (backlog 015) — the
production reviewer config scored against the live Threshold arena, with
pass^k task consistency across repeated trials:

```sh
cargo run -p crucible -- run evals/cerberus-review-quality-v0.json \
  --out runs/local/cerberus-review-quality \
  --json
```

`pass^k` (`k` = trials per task) only reports when every selected task shares
the same trial count ≥ 2 — it Wilson-scores the fraction of tasks where *every*
trial fully matched the adjudicated key (zero missed, zero false positives).
The independence unit is the task, not the trial — the same pattern
`crucible dashboard`'s leaderboard already used for `solve_rate`. This is a
real measurement of consistency, not just of average recall: a config that is
80% correct on average but never fully correct twice in a row reports a *low*
pass^k even with a decent key-recall score.

Query the ledger:

```sh
cargo run -p crucible -- runs list \
  --benchmark prompt-smoke-v0 \
  --json

cargo run -p crucible -- runs show <RUN_ID> --json

cargo run -p crucible -- runs compare \
  --benchmark prompt-smoke-v0 \
  --left openrouter/auto \
  --right openrouter/auto \
  --json
```

`runs compare` is intentionally descriptive: latest matching run per
config/model, Wilson intervals shown, no significance claim.

`runs show --json` returns both normalized rows and durable artifacts:
`run_record` (`crucible.run_record.v1`) plus `evaluation_card`
(`crucible.evaluation_card.v1`). Use this to inspect the persisted
reproducibility card for a run without scraping `prompt-run.json`.

**Backup/restore:** the ledger is one file, `runs/local/crucible-runs.sqlite`
by default (`--db <PATH>` for any other location) — fully gitignored, so
backing it up is not a repo concern. Copy the file while no `crucible run`/
`crucible adjudication-panel --serve` process has it open (SQLite does not
guarantee a consistent snapshot mid-write; a plain `cp` while a writer holds
the connection can copy a torn page). To restore, stop any writer and replace
the file with the backup copy — `open_initialized`'s schema init
(`CREATE TABLE IF NOT EXISTS`, see `run_store.rs`) is idempotent, so the next
`crucible run` reopens a restored file exactly like any other populated
ledger, no migration step. This is deliberately a documentation note, not new
backup infrastructure: real automated backup (e.g. Canary's Litestream
pattern) is an operator-scoped infra decision for if/when this ledger holds
data worth losing sleep over, not something to stand up unilaterally.

Run a Cerberus producer handoff through the same declared runner:

```sh
# from ../cerberus (sibling checkout)
target/debug/cerberus review \
  --request fixtures/requests/diff-only.json \
  --harness fixture \
  --fixture-output fixtures/harness/valid-review.txt \
  --out target/cerberus/crucible-live/artifact.json \
  --markdown target/cerberus/crucible-live/review.md \
  --execution-plan target/cerberus/crucible-live/execution_plan.json \
  --receipt-bundle target/cerberus/crucible-live/receipt-bundle.json
```

Then run a Crucible spec with `runner.corpus.source =
"cerberus_receipt_bundles"`. Each task must name `artifact`, `receipt_bundle`,
and the Harbor `tests/expected.json` scorer key. See
`crucible/tests/fixtures/specs/cerberus-receipt-fixture.json` and
`crucible/tests/fixtures/specs/cold-agent-smoke-v0.json` for two independently
authored committed shapes; keep real producer artifacts and specs under
`runs/local/`. Two things a real Cerberus producer already gets right but a
hand-authored fixture must set explicitly: `receipt_bundle.validation.status`
must be exactly `"passed"` (any other value refuses the run —
Crucible only grades trusted receipts), and `receipt_bundle.artifact_uri`
must match the spec's own `task.artifact` string (or resolve to the same
file) so Crucible can confirm the receipt actually vouches for the artifact
it is paired with. Both are the only fields `key_recall`'s
`cerberus_receipt_bundles` path validates beyond `schema_version`; this
runner (unlike `prompt_benchmark`) makes no network call, so a
hand-authored fixture spec runs fully hermetically.

```sh
cargo run -p crucible -- run --out runs/local/factory-lane --json
```

The report is written to `runs/local/factory-lane/run-report.json`.

The three concrete receipts are:

- `code-review-deterministic-floor`: Cerberus fixture vs Threshold/Daedalus
  `tests/expected.json`, scored as category-strict recall with a Wilson interval.
- `recoverable-adjudication-queue`: co-located category mismatch routed into the
  queue as a recoverable item, with a static phone panel artifact.
- `harbor-export-acceptance`: labeled queue exported to Harbor
  `adjudications.md`, `solution/findings.json`, and `tests/expected.json`.

Run one receipt when debugging:

```sh
cargo run -p crucible -- run \
  --eval recoverable-adjudication-queue \
  --out runs/local/recoverable-queue \
  --json
```

## Agent/MCP Surface

Serve the same run surface over stdio MCP:

```sh
cargo run -p crucible -- mcp
```

Call the `crucible_run` tool with either a declared spec:

```json
{
  "spec": "evals/pr-review-key-recall-v0.json"
}
```

Prompt benchmark example:

```json
{
  "spec": "evals/prompt-smoke-v0.json",
  "out": "runs/local/prompt-smoke-mcp"
}
```

or a built-in receipt selector:

```json
{
  "eval": "recoverable-adjudication-queue",
  "out": "runs/local/recoverable-queue"
}
```

The tool returns `content[0].text` as pretty `crucible.run_report.v1` JSON and
`structuredContent.report` as the same parsed object. It also writes
`run-report.json` under the reported output directory and stores the run in the
SQLite ledger. Use this surface when a human, agent, Threshold loop, or MCP
client needs to invoke evals directly.

Local browser workbench:

```sh
cargo run -p crucible -- serve \
  --db runs/local/crucible-runs.sqlite \
  --specs evals \
  --port 4174
```

`crucible serve` binds `127.0.0.1`. Reading the run ledger, launching runs, and
now applying adjudication labels (crucible-031) all require a bearer token: set
`CRUCIBLE_SERVE_TOKEN` and send `Authorization: Bearer <token>`, or those
routes 401. The read-only spec library (`GET /` and `GET /api/specs`) stays
open. Still expose this only through the private Bastion/Sanctum layer or
another authenticated local proxy — the bearer token is a same-machine
courtesy gate, not a substitute for network isolation.

Query tools:

- `crucible_validate`: check a declared spec's `{valid, runnable, errors,
  warnings}` before spending a `crucible_run` call on it — call this first.
- `crucible_grade`: score a Cerberus artifact against a Daedalus answer key —
  the same computation `crucible grade --json` emits.
- `crucible_adjudicate`: grade an artifact and build the adjudication queue,
  optionally applying labels from an `--apply`-shaped JSON array — the same
  computation `crucible adjudicate --json` emits. Use this to drive a headless
  labeling loop mid-agent-run instead of `adjudication-panel --serve`.
- `crucible_export`: turn a labeled judgment queue into the Daedalus
  key-extension artifacts (`adjudications.md`, and `solution/findings.json`/
  `tests/expected.json` when `key`/`expected` are given) — the same
  computation `crucible export` performs, writes and all.
- `crucible_runs_list`: list stored run rows, optionally filtered by
  benchmark, config id, model slug, or creation date.
- `crucible_runs_show`: fetch one run by run id with artifact pointers and
  indexed prompt task rows.
- `crucible_runs_compare`: compare the latest stored runs for two config ids
  or model slugs under one benchmark — pairs on shared prompt-task fixtures
  (McNemar) when both runs indexed the same task ids, falls back to an
  unpaired descriptive delta otherwise.

## Headless Eval Loop

Adapt a review artifact:

```sh
cargo run -p crucible -- adapt crucible/tests/fixtures/cerberus-artifact.json --json
```

Grade it against a scorer key:

```sh
cargo run -p crucible -- grade \
  --artifact crucible/tests/fixtures/cerberus-artifact.json \
  --key crucible/tests/fixtures/expected-defects.json \
  --json
```

Build the adjudication queue:

```sh
cargo run -p crucible -- adjudicate \
  --artifact crucible/tests/fixtures/cerberus-artifact.json \
  --key crucible/tests/fixtures/key-colocated-other-category.json \
  --json
```

Apply labels and export Harbor artifacts:

```sh
mkdir -p runs/local

cargo run -p crucible -- adjudicate \
  --artifact crucible/tests/fixtures/cerberus-artifact.json \
  --key crucible/tests/fixtures/key-colocated-other-category.json \
  --apply crucible/tests/fixtures/labels-keep-f1.json \
  --json > runs/local/labeled-queue.json

cargo run -p crucible -- export \
  --labels runs/local/labeled-queue.json \
  --out runs/local/harbor-export \
  --arena pr-review-v0 \
  --task py-file-cache \
  --base-version 0.2.0 \
  --date 2026-07-01 \
  --key crucible/tests/fixtures/key.json \
  --expected crucible/tests/fixtures/expected-defects.json
```

## Human Queue Surface

Render a static phone-first panel from an existing queue artifact:

```sh
cargo run -p crucible -- adjudication-panel \
  --queue crucible/tests/fixtures/export-queue.json \
  --out runs/local/adjudication-panel
```

Open `runs/local/adjudication-panel/index.html` to inspect the queue.

Or serve the panel with real writeback (backlog 005/012): a small local HTTP
server, no framework, that actually persists a Keep/Nit/Wrong/Noise tap:

```sh
cargo run -p crucible -- adjudication-panel \
  --queue crucible/tests/fixtures/export-queue.json \
  --out runs/local/adjudication-panel \
  --serve --port 4173
```

Open `http://127.0.0.1:4173/` and tap a verdict; each tap `POST`s to
`/label`, mints a `Label` through the same `apply_label` path `--apply` uses,
and persists the accumulated labels to `runs/local/adjudication-panel/labels.json`
(override with `--labels`) as a `crucible.label.v1` JSON array — the exact
shape `crucible adjudicate --apply <that file>` reads back, so a served
session re-enters the headless loop with zero conversion. Resumable: restart
`--serve` against the same `--labels` path and prior work is still there.

Live writeback is also mounted directly inside `crucible serve` (crucible-031)
— no separate `--serve` process required. `GET /adjudication/panel/<run_id>`
(bearer-protected, same as every other run-reading route) renders the live
panel for any run whose ledger row carries a real `queue.json` artifact, with
verdict taps posting to that run's own `POST
/adjudication/panel/<run_id>/label` route. It mints and persists through the
identical `apply_label` path and `crucible.label.v1` shape the standalone
`--serve` loop uses — `labels.json` sibling to the run's `queue.json` — so
`crucible adjudicate --apply` reads either one back the same way. `GET
/api/adjudication` lists which runs have a panel to open.

## Dashboard

```sh
cargo run -p crucible -- dashboard \
  --arenas ../daedalus/arenas \
  --runs ../daedalus/runs \
  --out runs/local/dashboard
```

Use the dashboard for read-side Threshold/Daedalus run evidence. It reports
bootstrap reward intervals, Wilson solve-rate intervals, and noise-floor
verdicts.

## Gate

Before claiming done:

```sh
./scripts/check.sh
```

Report the exact eval command, score, interval, output artifact path, gate
result, and any residual unverified path.
