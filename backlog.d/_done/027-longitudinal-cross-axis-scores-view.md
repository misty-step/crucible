# Longitudinal scores view across model/config/prompt/harness/tool axes

Priority: P1 · Status: done · Estimate: M

## Outcome (completed 2026-07-04)

`run_records` gains nullable `harness`/`tool_allowlist` columns (additive
`ALTER TABLE` migration via a generalized `ensure_column`, extending the
existing `task_class` precedent); `PromptModelConfig`/`AgenticJudgeConfig`
gain matching optional fields threaded through evidence JSON and `crucible
author --prompt-harness`/`--prompt-tool`. `config_id` gains an additive
`:harness=`/`:tools=` suffix only when recorded, so pre-existing config
identities are unchanged. `run_store::score_history` (CLI `runs history`,
MCP `crucible_runs_history`, `GET /api/history`) returns one config's score
trend oldest-to-newest. `run_store::pivot_by_model` (CLI `runs pivot`, MCP
`crucible_runs_pivot`, `GET /api/pivot`) returns one benchmark's latest run
per model, optionally narrowed to one harness. `runs list`/`crucible_runs_list`
gain a `--harness`/`harness` filter. No `011`/`009` regression: full
`cargo test --all` and `./scripts/check.sh` green.

## Goal

Close the gap between the operator's stated want — "a growing DB of runs/scores
across models, configs, parameters, system prompts, harnesses, tools
available" — and what the run database (`011`) and dashboard (`009`) actually
expose today. Most of the storage exists; the query/view surface for two of
those axes does not.

## What already exists (do not re-scope)

- `011` schema/write/query slice is done: every run persists model, provider,
  config id (composition hash), system-prompt hash, task results, score, and
  artifact pointers; `runs list` filters by benchmark/config/model/date;
  `runs compare` runs paired McNemar over shared tasks.
- `009` (dashboard) already renders per-config results sorted by score with
  seed-invariant, power-honest verdicts, and lists "trend sparklines across
  arena versions" as a named-but-undone next step.

## The actual gap

- **Harness and tool-availability are not first-class config dimensions.**
  `run_store.rs`'s config id is composed from `{runner, provider, model,
  system_prompt_hash}` (`run_store.rs:787`) — there is no field for which
  agent harness ran the candidate (pi, Claude Code, Codex, ...) or which tools
  were on the allowlist. Two runs on identical model/prompt but different
  harness or tool surface collapse into the same config id today, which
  breaks exactly the cross-harness/cross-tool comparison the operator asked
  for.
- **No time-axis query.** `runs list --since/--until` filters by run
  creation date, but there is no "show me benchmark X's score trend over time
  for config Y" query or view — `009`'s trend-sparkline item is the closest
  named work and is still undone.
- **No cross-axis pivot.** The dashboard shows one benchmark's configs
  ranked; there is no view that holds one axis (e.g. model) fixed and varies
  another (e.g. harness) over time, which is what "across models, configs,
  parameters, system prompts, harnesses, tools available" actually asks for
  as a single growing table, not five separate one-off queries.

## Oracle

- [x] Config identity gains explicit `harness` and `tool_allowlist` fields
  (or a documented equivalent) alongside the existing runner/provider/model/
  system-prompt-hash tuple, with a migration note for existing rows.
- [x] A time-series query (CLI + MCP) returns a benchmark's score history for
  a given config, ordered by run date, with intervals — the `009` trend-
  sparkline item, actually shipped and wired into the dashboard.
- [x] At least one cross-axis pivot view exists (e.g. "this benchmark, this
  model, every harness, over time" or "this benchmark, every model, this
  harness, over time") backed by the run database, not a hand-assembled
  report.
- [x] No existing `011`/`009` functionality regresses; this ticket extends the
  schema and view layer, it does not re-litigate the storage design.

## Notes

Report cross-reference: `ai-evals-benchmarks-report.md` §11 ("What to put in
an eval report") lists target system versions, tool definitions, and prompt
versions as required report fields — light confirmation that harness/tool
identity belongs in the run record, not new information; the gap here was
found by reading Crucible's own schema against the operator's verbatim ask,
not by the report.
