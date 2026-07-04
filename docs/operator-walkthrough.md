# Crucible in 10 minutes

This is the shortest useful path through Crucible: define a tiny benchmark,
run it against two OpenRouter models, and read the result without pretending a
five-task sample proves more than it does.

The benchmark below is intentionally small and plain. It has five prompt tasks,
one deterministic exact-match rubric, two model runs, one stored ledger, and one
UI receipt view. Raw model output stays under `runs/`, which is gitignored.

If a term below (`EvalSpec`, Wilson interval, noise floor, ...) is unfamiliar,
[`docs/glossary.md`](glossary.md) defines it in plain language — but you don't
need it to follow this walkthrough end to end.

## What you will build

You will create `evals/operator-micro-benchmark-v0.json`, a five-task prompt
benchmark:

| Task | What it checks | Expected answer |
| --- | --- | --- |
| `extract-ticket-id` | Pull an id out of text | `CRU-101` |
| `csv-largest-count` | Pick the max row in a CSV | `beta` |
| `json-owner-email` | Read one nested JSON field | `mira@example.test` |
| `sum-integers` | Add two integers | `105` |
| `date-to-iso` | Normalize a date | `2026-07-04` |

Crucible sends each task to each model, grades the final text with the exact
rubric, writes JSON evidence, stores rows in SQLite, and shows the rows in the
browser workbench.

## 0. Start clean, then verify the install is actually live

Run from the repo root:

```sh
pwd
git status --short --branch --untracked-files=all
cargo run -p crucible -- doctor --json
```

`doctor` (crucible-911) is the one command that replaces guessing whether the
CLI, MCP, serve UI, and run ledger actually work: it spawns the CLI, initializes
the MCP server and lists its tools, binds `crucible serve` to a real port and
checks `/api/specs`, and creates a scratch SQLite ledger under `runs/` — no
network call involved. Its `model_credentials` check separately reports
whether `OPENROUTER_API_KEY` is set (`warn`, not `fail`, when absent — the
value is never printed). If `doctor` exits non-zero, something in `checks` is
genuinely broken; fix that before continuing. If it exits 0 but
`model_credentials` is `warn`, the walkthrough's live-model steps below (1
onward) will not work until the key is loaded into the environment — do not
print it.

Representative `doctor --json` output (from a real run with `OPENROUTER_API_KEY`
set — every field except `ok`/`status` is diagnostic detail, not something to
match byte-for-byte):

```json
{
  "schema_version": "crucible.doctor_report.v1",
  "ok": true,
  "checks": [
    { "id": "cli", "status": "ok", "message": ".../crucible --version: crucible 0.0.0" },
    { "id": "mcp", "status": "ok", "message": "stdio MCP server initialized and listed 8 tool(s): ..." },
    { "id": "serve", "status": "ok", "message": "bound 127.0.0.1:56203 and GET /api/specs returned schema Some(\"crucible.ui.specs.v1\")" },
    { "id": "ledger", "status": "ok", "message": "created and opened runs/doctor-check/ledger-check-32029-1.sqlite (0 row(s), schema initialized)" },
    { "id": "model_credentials", "status": "ok", "message": "OPENROUTER_API_KEY is set (value not shown) — live-model prompt_benchmark/agentic_judge runs can reach OpenRouter" }
  ]
}
```

With no `OPENROUTER_API_KEY` set, every field is identical except
`model_credentials.status` becomes `"warn"` and `ok` stays `true` — a missing
optional credential never fails the doctor's overall verdict.

If the key is missing, stop and load it into the environment. Do not print it.

## 1. Define the benchmark

Write the benchmark spec:

```sh
cat > evals/operator-micro-benchmark-v0.json <<'JSON'
{
  "schema_version": "crucible.eval_spec.v1",
  "id": "operator-micro-benchmark-v0",
  "task": "operator-micro-benchmark",
  "inputs": "Five tiny exact-answer prompt tasks: structured extraction, CSV selection, JSON field lookup, arithmetic, and date normalization.",
  "outputs": "Model text graded by deterministic exact-match rubrics. No judge model is called.",
  "graders": {
    "graders": [
      {
        "id": "exact_match",
        "kind": "deterministic"
      }
    ]
  },
  "aggregation": "proportion",
  "uncertainty": {
    "method": "wilson",
    "confidence": 0.95
  },
  "decision": "Show a cold operator how to define, run, compare, and inspect a tiny Crucible benchmark in about ten minutes.",
  "runner": {
    "kind": "prompt_benchmark",
    "corpus": {
      "source": "prompt_benchmark",
      "config": {
        "provider": "open_router",
        "model": "deepseek/deepseek-v4-flash",
        "system_prompt": "You are running an exact-match benchmark. Output exactly the requested answer and nothing else. No markdown, no quotes unless they are part of the requested answer, no explanation.",
        "credential_env": "OPENROUTER_API_KEY",
        "max_tokens": 128,
        "temperature": 0
      },
      "tasks": [
        {
          "task_id": "extract-ticket-id",
          "prompt": "Fixed input: ticket=[CRU-101] status=ready. Return exactly the ticket id without brackets.",
          "expectation": {
            "kind": "exact",
            "value": "CRU-101"
          }
        },
        {
          "task_id": "csv-largest-count",
          "prompt": "Given CSV rows name,count\\nalpha,7\\nbeta,11\\ngamma,9\\nReturn exactly the name with the largest count.",
          "expectation": {
            "kind": "exact",
            "value": "beta"
          }
        },
        {
          "task_id": "json-owner-email",
          "prompt": "From this JSON, return exactly owner.email: {\"owner\":{\"name\":\"Mira\",\"email\":\"mira@example.test\"},\"repo\":\"crucible\"}",
          "expectation": {
            "kind": "exact",
            "value": "mira@example.test"
          }
        },
        {
          "task_id": "sum-integers",
          "prompt": "Return exactly the integer sum of 47 and 58.",
          "expectation": {
            "kind": "exact",
            "value": "105"
          }
        },
        {
          "task_id": "date-to-iso",
          "prompt": "Reformat exactly this date from July 4, 2026 to ISO YYYY-MM-DD.",
          "expectation": {
            "kind": "exact",
            "value": "2026-07-04"
          }
        }
      ]
    }
  }
}
JSON
```

Output: no terminal output on success.

## 2. Validate before spending model calls

```sh
cargo run -p crucible -- validate evals/operator-micro-benchmark-v0.json --json
```

Output:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.50s
     Running `target/debug/crucible validate evals/operator-micro-benchmark-v0.json --json`
{
  "schema_version": "crucible.validate_report.v1",
  "spec": "evals/operator-micro-benchmark-v0.json",
  "valid": true,
  "runnable": true,
  "errors": [],
  "warnings": []
}
```

Read that as: the JSON loads, the runner kind is supported, the grader kind is
actually executed, and the uncertainty rule is one Crucible really computes.

## 3. Run two models

Use an isolated output directory and SQLite ledger so this tutorial is easy to
delete or rerun:

```sh
cargo run -p crucible -- run evals/operator-micro-benchmark-v0.json \
  --models deepseek/deepseek-v4-flash,z-ai/glm-5.2 \
  --out runs/local/crucible-101/final \
  --db runs/local/crucible-101/final.sqlite \
  --json
```

Output:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running `target/debug/crucible run evals/operator-micro-benchmark-v0.json --models deepseek/deepseek-v4-flash,z-ai/glm-5.2 --out runs/local/crucible-101/final --db runs/local/crucible-101/final.sqlite --json`
{
  "schema_version": "crucible.run_fanout.v1",
  "db": "runs/local/crucible-101/final.sqlite",
  "runs": [
    {
      "model": "deepseek/deepseek-v4-flash",
      "output_dir": "runs/local/crucible-101/final/deepseek-deepseek-v4-flash",
      "run_report": "runs/local/crucible-101/final/deepseek-deepseek-v4-flash/run-report.json",
      "invocation_id": "run-1783193709912-13288-0",
      "run_records": 1,
      "prompt_task_results": 5
    },
    {
      "model": "z-ai/glm-5.2",
      "output_dir": "runs/local/crucible-101/final/z-ai-glm-5.2",
      "run_report": "runs/local/crucible-101/final/z-ai-glm-5.2/run-report.json",
      "invocation_id": "run-1783193718473-13288-1",
      "run_records": 1,
      "prompt_task_results": 5
    }
  ]
}
```

This is the first important receipt. Each model produced one run record and
five prompt-task rows.

## 4. List the stored runs

```sh
cargo run -p crucible -- runs list \
  --benchmark operator-micro-benchmark-v0 \
  --db runs/local/crucible-101/final.sqlite \
  --json
```

Output:

```json
{
  "schema_version": "crucible.run_store.v1",
  "db": "runs/local/crucible-101/final.sqlite",
  "benchmark": "operator-micro-benchmark-v0",
  "runs": [
    {
      "run_id": "run-1783193718473-13288-1:operator-micro-benchmark-v0",
      "benchmark_id": "operator-micro-benchmark-v0",
      "runner_kind": "prompt_benchmark",
      "model": "z-ai/glm-5.2",
      "successes": 4,
      "n": 5,
      "point": 0.8,
      "lower": 0.3755282641185388,
      "upper": 0.9637768390302125,
      "confidence": 0.95,
      "evidence_path": "runs/local/crucible-101/final/z-ai-glm-5.2/prompt-run.json"
    },
    {
      "run_id": "run-1783193709912-13288-0:operator-micro-benchmark-v0",
      "benchmark_id": "operator-micro-benchmark-v0",
      "runner_kind": "prompt_benchmark",
      "model": "deepseek/deepseek-v4-flash",
      "successes": 5,
      "n": 5,
      "point": 1.0,
      "lower": 0.5655085052479191,
      "upper": 1.0,
      "confidence": 0.95,
      "evidence_path": "runs/local/crucible-101/final/deepseek-deepseek-v4-flash/prompt-run.json"
    }
  ]
}
```

The list is newest first. The `point` is the pass rate. The `lower` and `upper`
fields are the Wilson 95% interval. With only five tasks, those intervals are
wide, which is correct.

## 5. Compare the two runs

```sh
cargo run -p crucible -- runs compare \
  --benchmark operator-micro-benchmark-v0 \
  --left deepseek/deepseek-v4-flash \
  --right z-ai/glm-5.2 \
  --db runs/local/crucible-101/final.sqlite \
  --json
```

Output:

```json
{
  "schema_version": "crucible.run_store.v1",
  "db": "runs/local/crucible-101/final.sqlite",
  "benchmark": "operator-micro-benchmark-v0",
  "left_query": "deepseek/deepseek-v4-flash",
  "right_query": "z-ai/glm-5.2",
  "left": {
    "model": "deepseek/deepseek-v4-flash",
    "successes": 5,
    "n": 5,
    "point": 1.0,
    "lower": 0.5655085052479191,
    "upper": 1.0
  },
  "right": {
    "model": "z-ai/glm-5.2",
    "successes": 4,
    "n": 5,
    "point": 0.8,
    "lower": 0.3755282641185388,
    "upper": 0.9637768390302125
  },
  "delta_point": -0.19999999999999996,
  "common_tasks": 5,
  "paired": {
    "b": 1,
    "c": 0,
    "statistic": 0.0,
    "p_value": 1.0,
    "verdict": "inside_noise_floor"
  },
  "comparison_kind": "paired_mcnemar",
  "note": "Paired McNemar comparison over prompt tasks common to both runs; see paired.verdict for the noise-floor decision."
}
```

Read that as: DeepSeek passed 5/5 and GLM passed 4/5, but the one-task
difference does not clear the noise floor. This is Crucible doing its job. It
reports the score and refuses to overstate the delta.

## 6. Read the per-task verdicts

The run evidence keeps the raw model outputs and the rubric verdict for each
task:

```sh
jq '{model, score, totals, tasks: [.tasks[] | {task_id, passed, output, response_model, latency_ms, cost_usd}]}' \
  runs/local/crucible-101/final/deepseek-deepseek-v4-flash/prompt-run.json
```

Output:

```json
{
  "model": "deepseek/deepseek-v4-flash",
  "score": {
    "successes": 5,
    "n": 5,
    "point": 1.0,
    "lower": 0.5655085052479191,
    "upper": 1.0,
    "confidence": 0.95
  },
  "totals": {
    "tasks": 5,
    "passed": 5,
    "failed": 0
  },
  "tasks": [
    {
      "task_id": "extract-ticket-id",
      "passed": true,
      "output": "CRU-101",
      "response_model": "deepseek/deepseek-v4-flash-20260423",
      "latency_ms": 3591,
      "cost_usd": 0.000009869
    },
    {
      "task_id": "csv-largest-count",
      "passed": true,
      "output": "beta",
      "response_model": "deepseek/deepseek-v4-flash-20260423",
      "latency_ms": 1249,
      "cost_usd": 0.00000925
    },
    {
      "task_id": "json-owner-email",
      "passed": true,
      "output": "mira@example.test",
      "response_model": "deepseek/deepseek-v4-flash-20260423",
      "latency_ms": 1945,
      "cost_usd": 0.00002603
    },
    {
      "task_id": "sum-integers",
      "passed": true,
      "output": "105",
      "response_model": "deepseek/deepseek-v4-flash-20260423",
      "latency_ms": 1201,
      "cost_usd": 0.000014812
    },
    {
      "task_id": "date-to-iso",
      "passed": true,
      "output": "2026-07-04",
      "response_model": "deepseek/deepseek-v4-flash-20260423",
      "latency_ms": 720,
      "cost_usd": 0.0000084
    }
  ]
}
```

Now inspect the GLM evidence:

```sh
jq '{model, score, totals, tasks: [.tasks[] | {task_id, passed, output, response_model, latency_ms, cost_usd}]}' \
  runs/local/crucible-101/final/z-ai-glm-5.2/prompt-run.json
```

Output:

```json
{
  "model": "z-ai/glm-5.2",
  "score": {
    "successes": 4,
    "n": 5,
    "point": 0.8,
    "lower": 0.3755282641185388,
    "upper": 0.9637768390302125,
    "confidence": 0.95
  },
  "totals": {
    "tasks": 5,
    "passed": 4,
    "failed": 1
  },
  "tasks": [
    {
      "task_id": "extract-ticket-id",
      "passed": false,
      "output": "",
      "response_model": "z-ai/glm-5.2-20260616",
      "latency_ms": 3720,
      "cost_usd": 0.00046494
    },
    {
      "task_id": "csv-largest-count",
      "passed": true,
      "output": "beta",
      "response_model": "z-ai/glm-5.2-20260616",
      "latency_ms": 984,
      "cost_usd": 0.0001434
    },
    {
      "task_id": "json-owner-email",
      "passed": true,
      "output": "mira@example.test",
      "response_model": "z-ai/glm-5.2-20260616",
      "latency_ms": 1533,
      "cost_usd": 0.00013238
    },
    {
      "task_id": "sum-integers",
      "passed": true,
      "output": "105",
      "response_model": "z-ai/glm-5.2-20260616",
      "latency_ms": 895,
      "cost_usd": 0.000874
    },
    {
      "task_id": "date-to-iso",
      "passed": true,
      "output": "2026-07-04",
      "response_model": "z-ai/glm-5.2-20260616",
      "latency_ms": 1423,
      "cost_usd": 0.00014322
    }
  ]
}
```

The failed task is not hidden. GLM returned empty text for
`extract-ticket-id`, so the exact-match verifier marked that row failed.

## 7. Read it in the UI

Start the local workbench on the same isolated ledger:

```sh
cargo run -p crucible -- serve \
  --db runs/local/crucible-101/final.sqlite \
  --specs evals \
  --port 4174
```

Output:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.29s
     Running `target/debug/crucible serve --db runs/local/crucible-101/final.sqlite --specs evals --port 4174`
crucible serve: http://127.0.0.1:4174
```

Open `http://127.0.0.1:4174/`.

Then:

1. On **Benchmarks**, find `operator-micro-benchmark`. The card should say
   `5 tasks`, `ready`, and show the latest result from this ledger.
2. Click **Receipts**.
3. Click the `z-ai/glm-5.2` row.
4. In **Task results**, read each row. The `extract-ticket-id` row is `fail`;
   the other four rows are `pass`.
5. Click the artifact links if you want the exact JSON evidence.

The UI is a readback of the same SQLite ledger and evidence files you inspected
from the CLI. It is not a separate scoring system.

## What this proved

- You authored an executable benchmark as data, not a one-off script.
- `crucible validate` confirmed the spec is runnable before model spend.
- `crucible run --models` called two real OpenRouter models.
- Crucible wrote raw model evidence under `runs/`.
- Crucible stored run rows and task rows in SQLite.
- CLI and UI read the same evidence.
- The comparison showed a visible score gap but refused a significance claim
  because five tasks is too small.

## What this did not prove

- It does not rank these models generally.
- It does not prove GLM is worse than DeepSeek.
- It does not exercise human adjudication or agentic judge calibration.
- It does not prove the deployed Bastion instance has picked up this new spec.
  That deployment/readback follow-up is tracked as Powder card `crucible-903`.
