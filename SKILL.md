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

Use an isolated ledger for tests or one-off proof:

```sh
cargo run -p crucible -- run evals/prompt-smoke-v0.json \
  --out runs/local/prompt-smoke \
  --db runs/local/crucible-runs.sqlite \
  --json
```

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
`crucible/tests/fixtures/specs/cerberus-receipt-fixture.json` for the committed
shape; keep real producer artifacts and specs under `runs/local/`.

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

Query tools:

- `crucible_runs_list`: list stored run rows, optionally filtered by benchmark.
- `crucible_runs_show`: fetch one run by run id with artifact pointers and
  indexed prompt task rows.
- `crucible_runs_compare`: compare latest stored runs for two config ids or
  model slugs under one benchmark.

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
