# Crucible

Crucible is the eval and benchmark workbench for Misty Step's AI and agent
work. It designs, runs, judges, calibrates, reports, and exports evals that
survive contact with real agent behavior.

Its one principle: **refuse to report a delta it cannot defend**. Every rate
carries an interval, every judge needs calibration before it is trusted, and
rank gaps inside the noise floor are reported as inconclusive.

For the project north star and the boundary with Daedalus and Harness Kit, read
[`VISION.md`](VISION.md). For the cold-agent command contract, read
[`SKILL.md`](SKILL.md).

## Current State

Crucible now has a Rust core and CLI for the first eval family: agentic
code-review quality. The shipped wedge can:

- execute a declared `EvalSpec` with `crucible run <spec>`;
- adapt Cerberus review artifacts into Daedalus answer-key rows;
- grade findings against either `solution/findings.json` or
  `tests/expected.json`;
- build and label a `crucible.judgment_queue.v1` adjudication queue;
- export accepted findings back into Harbor/Daedalus scorer artifacts;
- render a phone-first eval dashboard over Daedalus arenas/runs;
- run three committed eval receipts with defensible Wilson intervals;
- render a static phone-first adjudication panel from an existing queue artifact.

Raw eval run records belong under `runs/`, which is gitignored because real runs
can embed proprietary diffs, raw model outputs, and API-keyed transcripts.
Committed fixture inputs live only under `crucible*/tests/fixtures/`.

## Runnable Evals

Run the first declared benchmark spec:

```sh
cargo run -p crucible -- run evals/pr-review-key-recall-v0.json --json
```

That spec, `pr-review-key-recall-v0`, selects the frozen Daedalus
`pr-review-v0` trials corpus and grades the `probe-oneshot` candidate against
the Harbor scorer keys under the sibling Daedalus checkout. The default evidence
directory is `runs/local/pr-review-key-recall-v0/`.

Run a Cerberus producer handoff through the same declared-spec runner:

```sh
# from the sibling Cerberus checkout
target/debug/cerberus review \
  --request fixtures/requests/diff-only.json \
  --harness fixture \
  --fixture-output fixtures/harness/valid-review.txt \
  --out target/cerberus/crucible-live/artifact.json \
  --markdown target/cerberus/crucible-live/review.md \
  --execution-plan target/cerberus/crucible-live/execution_plan.json \
  --receipt-bundle target/cerberus/crucible-live/receipt-bundle.json
```

Then run a Crucible spec whose runner corpus uses
`"source": "cerberus_receipt_bundles"` and names the Cerberus `artifact`, the
`receipt_bundle`, and the Harbor `tests/expected.json` scorer key. The hermetic
fixture spec is
`crucible/tests/fixtures/specs/cerberus-receipt-fixture.json`; real run records
belong under `runs/local/`.

Run all built-in eval receipts:

```sh
cargo run -p crucible -- run --out runs/local/factory-lane --json
```

The command writes `runs/local/factory-lane/run-report.json` plus one evidence
directory per eval:

- `code-review-deterministic-floor`: grades the real Cerberus fixture against a
  Daedalus `tests/expected.json` scorer key and reports category-strict recall.
- `recoverable-adjudication-queue`: proves a co-located category mismatch is
  routed into the human queue as a recoverable adjudication item and renders the
  phone panel.
- `harbor-export-acceptance`: applies committed labels and exports the accepted
  finding into Harbor scorer/oracle artifacts.

Each score is binary and small-n by design, so its Wilson interval is wide. That
is the intended behavior: the eval is runnable evidence, not overclaimed
precision.

## CLI

```sh
cargo run -p crucible -- adapt crucible/tests/fixtures/cerberus-artifact.json --json

cargo run -p crucible -- grade \
  --artifact crucible/tests/fixtures/cerberus-artifact.json \
  --key crucible/tests/fixtures/expected-defects.json \
  --json

cargo run -p crucible -- adjudicate \
  --artifact crucible/tests/fixtures/cerberus-artifact.json \
  --key crucible/tests/fixtures/key-colocated-other-category.json \
  --json

cargo run -p crucible -- adjudication-panel \
  --queue crucible/tests/fixtures/export-queue.json \
  --out runs/local/adjudication-panel
```

`--json` outputs carry stable `schema_version` values and the CLI uses stable
exit codes: `0` success, `1` load/parse failure, `2` usage error.

## Dashboard

```sh
cargo run -p crucible -- dashboard \
  --arenas /Users/phaedrus/Development/daedalus/arenas \
  --runs /Users/phaedrus/Development/daedalus/runs \
  --out runs/local/dashboard
```

The dashboard writes a self-contained `index.html` and a stable `data.json`
model. It renders the measured leaderboard; it does not recompute statistics in
the browser.

## Backlog

- `001` — shaping (done; proposed for archive)
- `002` — code-review eval wedge (in progress; core loop shipped, judge
  calibration still needs human labels)
- `003` — measurement rigor core
- `004` — eval object and per-eval grader-mix model
- `005` — phone-first adjudication queue
- `006` — agent-readiness and machine surface
- `007` — extract eval-authoring from Daedalus
- `008` — make `crucible grade` predict Daedalus reward
- `009` — live eval dashboard

## Gate

The repo gate is:

```sh
./scripts/check.sh
```

It runs the credential leak scan, `cargo fmt`, clippy with `-D warnings`, tests,
build, and rustdoc with warnings denied. `make check` delegates to the same
script.
