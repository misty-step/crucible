# Persist every benchmark run in a queryable database

Priority: P0 · Status: ready · Estimate: L (epic)

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

1. Schema v1 — benchmark, run, config, task_result, artifact, score, provenance.
2. Write path from existing built-in receipts and declared specs.
3. Query path — CLI list/show/export plus MCP tools.
4. Dashboard read path for Crucible-owned runs.
5. Backup/restore note and migration tests.

## Notes

Operator decision 2026-07-01: "Runs database: SQLite (canary pattern) — every
run ever, any config, attached to its benchmark; queryable; this is what
Threshold consumes." Keep raw model outputs and real diffs under ignored
artifact paths; the database stores pointers and metadata unless content has
been explicitly redacted for publication.
