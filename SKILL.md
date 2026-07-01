---
name: crucible
description: Run and extend Crucible evals. Use when an agent needs to author, run, grade, adjudicate, export, or report Crucible code-review evals from this repo.
---

# Crucible Verification Skill

Crucible owns eval artifacts, judgment queues, uncertainty, and exports. The
first family is agentic code-review quality over Cerberus findings and Daedalus
Harbor scorer keys.

## Start Here

Read `VISION.md` and `AGENTS.md` before changing eval semantics, grader
boundaries, runner boundaries, exports, or UI. Raw model outputs and raw diffs
must stay under `runs/` unless deliberately committed as sanitized fixtures under
`crucible*/tests/fixtures/`.

## Run The Three Built-In Evals

Run the first declared benchmark spec:

```sh
cargo run -p crucible -- run evals/pr-review-key-recall-v0.json --json
```

The spec writes `runs/local/pr-review-key-recall-v0/run-report.json` by default.
It measures Daedalus `pr-review-v0` key recall for the selected
`probe-oneshot` candidate over the frozen six-task corpus.

```sh
cargo run -p crucible -- run --out runs/local/factory-lane --json
```

The report is written to `runs/local/factory-lane/run-report.json`.

The three concrete receipts are:

- `code-review-deterministic-floor`: Cerberus fixture vs Daedalus
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
  --arenas /Users/phaedrus/Development/daedalus/arenas \
  --runs /Users/phaedrus/Development/daedalus/runs \
  --out runs/local/dashboard
```

Use the dashboard for read-side Daedalus run evidence. It reports bootstrap
reward intervals, Wilson solve-rate intervals, and noise-floor verdicts.

## Gate

Before claiming done:

```sh
./scripts/check.sh
```

Report the exact eval command, score, interval, output artifact path, gate
result, and any residual unverified path.
