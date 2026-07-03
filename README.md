# Crucible

Crucible is the eval and benchmark workbench for Misty Step's AI and agent
work. It designs, runs, judges, calibrates, reports, and exports evals that
survive contact with real agent behavior.

Its one principle: **refuse to report a delta it cannot defend**. Every rate
carries an interval, every judge needs calibration before it is trusted, and
rank gaps inside the noise floor are reported as inconclusive.

For the project north star and the boundary with Threshold and Harness Kit, read
[`VISION.md`](VISION.md). For the cold-agent command contract, read
[`SKILL.md`](SKILL.md).

## Current State

Crucible now has a Rust core and CLI for the first eval family: agentic
code-review quality. The shipped wedge can:

- execute a declared `EvalSpec` with `crucible run <spec>`;
- run a Crucible-owned prompt benchmark through an OpenRouter-compatible BYOK
  model boundary and grade it with a deterministic rubric;
- expose the same run surface as a stdio MCP tool for agents and Threshold;
- persist each run into a gitignored SQLite ledger and query it by benchmark,
  run id, or latest config/model comparison through CLI and MCP;
- materialize durable `crucible.run_record.v1` and
  `crucible.evaluation_card.v1` JSON for stored runs;
- adapt Cerberus review artifacts into Threshold/Daedalus answer-key rows;
- grade findings against either `solution/findings.json` or
  `tests/expected.json`;
- build and label a `crucible.judgment_queue.v1` adjudication queue;
- export accepted findings back into Harbor scorer artifacts;
- render a phone-first eval dashboard over Threshold/Daedalus arenas/runs;
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

That spec, `pr-review-key-recall-v0`, selects the frozen Threshold
`pr-review-v0` trials corpus and grades the `probe-oneshot` candidate against
the Harbor scorer keys under the sibling `daedalus` checkout. The default
evidence directory is `runs/local/pr-review-key-recall-v0/`.
Every `crucible run` invocation also writes rows into the SQLite run ledger at
`runs/local/crucible-runs.sqlite` unless `--db <PATH>` is supplied.

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
  Threshold/Daedalus `tests/expected.json` scorer key and reports
  category-strict recall.
- `recoverable-adjudication-queue`: proves a co-located category mismatch is
  routed into the human queue as a recoverable adjudication item and renders the
  phone panel.
- `harbor-export-acceptance`: applies committed labels and exports the accepted
  finding into Harbor scorer/oracle artifacts.

Each score is binary and small-n by design, so its Wilson interval is wide. That
is the intended behavior: the eval is runnable evidence, not overclaimed
precision.

Run the first Crucible-owned prompt benchmark through OpenRouter:

```sh
OPENROUTER_API_KEY=... \
cargo run -p crucible -- run evals/prompt-smoke-v0.json \
  --out runs/local/prompt-smoke \
  --json
```

This spec makes a real model call through the OpenRouter chat-completions API,
captures token/latency metadata when the provider returns it, grades the response
with a deterministic contains rubric, and writes
`runs/local/prompt-smoke/prompt-run.json`. The same prompt task evidence is
indexed into the SQLite ledger as task rows plus artifact pointers. Raw model
output stays under `runs/`.

Run the deterministic tracer benchmark across selected real models:

```sh
for model in \
  deepseek/deepseek-v4-flash \
  z-ai/glm-5.2 \
  moonshotai/kimi-k2.7-code
do
  safe_model=$(printf '%s' "$model" | tr '/:.' '---')
  cargo run -p crucible -- run evals/tracer-exact-v0.json \
    --model "$model" \
    --out "runs/local/tracer-exact/$safe_model" \
    --json
done

cargo run -p crucible -- runs compare \
  --benchmark tracer-exact-v0 \
  --left deepseek/deepseek-v4-flash \
  --right z-ai/glm-5.2
```

`evals/tracer-exact-v0.json` is deliberately tiny: three exact-match prompt
tasks, deterministic grading only, no Threshold corpus, and no judge-model
calls. `--model` is a run-time override for declared `prompt_benchmark` specs;
it lets one committed eval artifact produce comparable run rows for multiple
OpenRouter model slugs.

Query the run ledger:

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

`runs compare` reports the latest stored run for each config id or model slug
and labels the result as a descriptive unpaired delta. It includes Wilson
intervals; it does not claim statistical significance.

`runs show --json` includes the normalized row, artifact pointers, indexed
prompt task rows when present, the stored `run_record`, and the nested
`evaluation_card` provenance. Prompt benchmark cards carry model/version,
configured temperature when explicit, prompt/rubric hashes, fixture refs, cost,
and timestamp; deterministic key-recall runs use explicit `deterministic`
provenance.

## MCP

Start the stdio MCP server from the repo root:

```sh
cargo run -p crucible -- mcp
```

The server exposes `crucible_run`, backed by the same implementation as
`crucible run`, plus `crucible_runs_list`, `crucible_runs_show`, and
`crucible_runs_compare` over the same SQLite ledger.

Declared spec example:

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

Built-in receipt example:

```json
{
  "eval": "recoverable-adjudication-queue",
  "out": "runs/local/recoverable-queue"
}
```

The MCP result includes the pretty `crucible.run_report.v1` text, structured
report content, the output directory, the written `run-report.json` path, and a
`run_store` receipt naming the persisted ledger rows.

## Local Serve

Run the local browser workbench:

```sh
cargo run -p crucible -- serve \
  --db runs/local/crucible-runs.sqlite \
  --specs evals \
  --port 4174
```

The server binds `127.0.0.1` only and prints the bound URL. It serves the
benchmark library, validation results, run ledger list/detail views, trends,
artifact readback, and `POST /api/run` over the same declared-spec runner used
by the CLI and MCP. Model-backed prompt runs require `OPENROUTER_API_KEY` in the
process environment; deterministic read paths do not.

Sanctum prepare-only posture:

- Build artifact: `cargo build --release -p crucible`, then run
  `target/release/crucible serve`.
- Process command:
  `OPENROUTER_API_KEY=<secret> target/release/crucible serve --db /var/lib/crucible/crucible-runs.sqlite --specs /srv/crucible/evals --port 4174`.
- Storage contract: keep the SQLite ledger and run artifacts on the box outside
  the git checkout; raw model outputs and diffs are not publishable assets.
- Network/auth contract: `crucible serve` has no app-level auth and binds
  localhost. Expose it only through the Bastion/Sanctum private tailnet layer or
  an authenticated reverse proxy; do not bind it publicly.
- Readiness probe: `GET /api/specs` should return
  `crucible.ui.specs.v1` with the mounted spec directory.

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
  --arenas ../daedalus/arenas \
  --runs ../daedalus/runs \
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
- `007` — extract eval-authoring from Threshold
- `009` — live eval dashboard
- `010` — author-and-run engine
- `011` — runs database
- `012` — three judge tiers real
- `013` — one scorer, one crate
- `014` — agent-first surfaces and honest specs
- `015` — first real Cerberus review-quality benchmark
- `016` — publicable hygiene and fleet integration

## Gate

The repo gate is:

```sh
./scripts/check.sh
```

It runs the credential leak scan, `cargo fmt`, clippy with `-D warnings`, tests,
build, and rustdoc with warnings denied. `make check` delegates to the same
script.
