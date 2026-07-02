# Persist every benchmark run in a queryable database

Priority: P0 · Status: in-progress · Estimate: L (epic)

## Goal

Create a Crucible-owned SQLite run database that stores every benchmark run,
its config, its benchmark version, and its evidence attachments so Threshold and
agents can query trusted run history instead of scraping loose artifacts.

## Oracle

- [ ] Every `crucible run` invocation writes a row for the run, benchmark,
  config, task results, score, uncertainty, cost, latency, and artifact pointers.
- [ ] Runs are queryable by benchmark, model/config, date, score verdict, and
  run id through CLI and MCP.
- [ ] The dashboard reads Crucible's run database for Crucible-owned runs while
  still accepting imported Threshold/Daedalus history during the migration.
- [ ] The database follows the Canary durability pattern: SQLite with a clear
  backup/restore story, no committed raw outputs, and no secret-bearing paths in
  tracked files.

## Verification System

- Claim: Crucible has a durable run ledger attached to benchmarks, not only
  per-command JSON reports.
- Falsifier: deleting `runs/local/<id>/run-report.json` leaves no queryable
  record, or a run cannot be traced back to benchmark/config/task evidence.
- Driver: `crucible run` over a fixture benchmark, then `crucible runs list/show`
  and MCP query calls against the SQLite file.
- Grader: integration tests over a temp database plus a live local run receipt.
- Evidence packet: database file in `runs/local/`, JSON query transcripts, and
  dashboard `data.json` proving the run renders from the database.
- Cadence: every run-store schema change.

## Children

1. ✅ Schema v1 — invocation, run, config, prompt task result, artifact, score,
   and full JSON persistence.
2. ✅ Write path from existing built-in receipts and declared specs via
   `crucible run --db <PATH>` and MCP `crucible_run`.
3. ✅ Query path. CLI and MCP `runs list` filter by benchmark, config id,
   model slug, and creation-date bounds (`--since`/`--until`); `runs compare`
   pairs on shared prompt task fixtures (McNemar via the existing
   `crucible-core` stats kernel) when both runs carry indexed task rows, and
   falls back to the unpaired descriptive delta otherwise. Export remains.
4. Dashboard read path for Crucible-owned runs.
5. ✅ Backup/restore note and migration tests — see
   `backlog.d/020-run-ledger-reopen-and-backup-doc.md`.

## Notes

Operator decision 2026-07-01: "Runs database: SQLite (canary pattern) — every
run ever, any config, attached to its benchmark; queryable; this is what
Threshold consumes." Keep raw model outputs and real diffs under ignored
artifact paths; the database stores pointers and metadata unless content has
been explicitly redacted for publication.

Progress 2026-07-01: schema/write/query slice landed. Default ledger path is
`runs/local/crucible-runs.sqlite` with `--db <PATH>` overrides for isolated
proof. `prompt-run.json` remains an artifact, but prompt task results are also
stored as rows with model, pass/fail, latency, usage, cost, output text, and
the original task JSON. `runs compare` is a latest-run descriptive delta with
intervals, not a significance claim.

Progress 2026-07-01: durable record materialization landed on top of the ledger.
Every new persisted run row gets a `run_record_materializations` row containing
`crucible.run_record.v1` plus the nested `crucible.evaluation_card.v1`; `runs
show` and MCP show expose both. Remaining: dashboard read path, export, and
backup/restore/migration notes.

Progress 2026-07-01: `runs list` gained `--config`, `--model`, `--since`, and
`--until` filters (CLI + MCP), backed by a `RunListFilter` over the same SQL
query. `runs compare` gained a paired path: when both sides' latest run carry
`prompt_task_results` rows and share at least one `task_id`, the comparison
runs `PairedComparison::mcnemar` (crucible-core's noise-floor kernel, already
used by the leaderboard) over the shared tasks and reports a `McnemarOutcome`
(`b`/`c`/statistic/p-value/verdict) plus `common_tasks`, gated by a
`--alpha`/`alpha` significance threshold (default `0.05`). Deterministic
runners with no indexed prompt tasks (e.g. `key_recall`) keep the prior
`latest_unpaired_descriptive_delta` behavior. Remaining: dashboard read path,
export, and backup/restore/migration notes.
