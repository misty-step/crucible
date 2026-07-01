# Author-and-run — Crucible defines and RUNS a benchmark end to end (the functional tracer bullet)

Priority: P0 · Status: in-progress · Estimate: L (epic)

## Goal

Crucible can DEFINE a benchmark (tasks + rubric) as its own artifact and RUN it
against a config (a real model call), grade the output, record a run, and show the
result — with **no dependency on Threshold to do the running**. This is the point of
Crucible: design benchmarks and run them against the configs we care about.

## The gap (why this is the real tracer bullet)

Today Crucible has the SCORING half (`grade`/`adjudicate`/`export`/`measure`), the
VIEWING half (the dashboard), and the type contract (`EvalSpec`/`RunRecord`/
`EvaluationCard`, epic 004). It is missing the ENGINE: a Crucible-owned benchmark on
disk, and a RUNNER that executes a config against a task by calling a model, capturing
output, grading it, and recording the run. **Crucible has never made a live model call
(0 HTTP/model deps in Cargo.toml).** Every run surfaced so far is Threshold's. This
epic makes Crucible produce its own.

## Oracle

- [ ] A Crucible-owned benchmark (≥1 task + a declared rubric) is defined on disk in
  Crucible's own format (`EvalSpec`), independent of Threshold's arenas.
- [ ] `crucible run --benchmark <b> --config <model+prompt>` executes the config
  against each task via a REAL model call, captures output + tokens + cost +
  latency, grades it with the rubric, and writes a `RunRecord`/`EvaluationCard`
  with per-task results.
- [ ] The same authored benchmark can be launched from CLI and MCP; the MCP
  surface is first-class, not a later wrapper.
- [ ] The run + its results appear in the dashboard (the Guided view), sourced from
  Crucible's OWN run store — not Threshold's.
- [ ] End-to-end proof: author a tiny benchmark, run it against a real model, get a
  real recorded score with its uncertainty, and see it — no Threshold in the loop.

## Children (ordered)

1. ✅ **Crucible-owned benchmark format on disk** — a concrete `EvalSpec` (task = prompt/
   context + input + rubric ref) + a starter benchmark with 1–2 tasks and a
   deterministic rubric. Reuse the 004 types; do not reinvent.
2. ✅ **The runner / harness** — a `run` module that executes a config (v0 = model id +
   system prompt; NO agent tools/loop yet) against a task = a live model call.
   BYOK/OpenRouter-compatible first unless a concrete model boundary requires a
   direct provider client; keys come from env/secret and are never logged or leaked
   (the leak gate must still pass). Capture output, tokens, cost, latency,
   errors; retry + timeout.
3. **Grade + record** — grade the model output with the task's rubric (deterministic
   first; model-judge/human later, behind 003's calibration gate). Write a per-task
   trial + a `RunRecord`/`EvaluationCard` (reuse 004 + 003 provenance).
4. **Crucible's own run store** — a Crucible-owned `runs/` tree (gitignored where it
   embeds real model output/diffs, per 006.3); wire the dashboard to read Crucible's
   runs (alongside or instead of Threshold's).
5. **`crucible run` CLI** — stable JSON + exit codes; the Guided dashboard surfaces the
   result.

## Verification System

- Claim: Crucible can author a benchmark and run it against a chosen config, producing
  an honest, recorded, viewable result with no Threshold dependency.
- Falsifier: the run can't make a real model call; or the score isn't recorded/
  reproducible; or it silently depends on Threshold to run.
- Driver: `crucible run` and `crucible_run` MCP over a Crucible-owned tiny
  benchmark + a real BYOK model config.
- Grader: deterministic rubric v0; the `measure` core for uncertainty.
- Evidence packet: a real `RunRecord`/`EvaluationCard` (model + tokens + cost + latency
  + score + prompt/rubric hash) visible in the dashboard.
- Cadence: per child; the end-to-end proof gates the epic.

## Notes

This introduces Crucible's FIRST live model call — a new capability and a new
dependency (an HTTP client, e.g. `reqwest`, plus a tiny provider adapter). Name
the boundary before coding (model-native product primitive): the model seam is
explicit; deterministic code owns policy, persistence, and grading. API keys via
env/secret only; the leak gate must not catch or leak them; run artifacts
embedding real outputs are gitignored/redacted (006.3).

v0 harness = model + system prompt only. Tools/agentic harness and multi-config runs
come later. The SEARCH over many configs is Threshold's job, not Crucible's — Crucible
runs the configs we choose and reports honest results.

**Supersedes 009's "wire the design in" as the priority:** the dashboard (009) + the
converged Guided design (served at `/crucible-redesign/guided/`) are the SHELL/VIEW;
this epic is the ENGINE. The Guided design becomes the view for these real runs.

**Update 2026-07-01:** first author-and-run slice landed: `prompt_benchmark`
runner kind, `evals/prompt-smoke-v0.json`, OpenRouter-compatible BYOK model
client, deterministic exact/contains text rubrics, `prompt-run.json` evidence,
and CLI + MCP live proof. Still open: durable `RunRecord`/`EvaluationCard`
persistence, runs database integration, dashboard Guided view, and broader
author/manage config surfaces.
