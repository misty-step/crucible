# Runs ledger: reopen-safety test + backup/restore doc note

Priority: P2 · Status: ready · Estimate: S

## Goal

Close backlog `011`'s remaining child 5 ("Backup/restore note and migration
tests"): prove the SQLite run ledger (`crucible/src/run_store.rs`,
`open_initialized`/`init_schema`, lines 586-703) survives being reopened
against an existing populated database file without data loss or schema
error, and document the backup story in `SKILL.md` (the file already
documents `--db <PATH>` overrides but says nothing about backing up or
restoring `runs/local/crucible-runs.sqlite`).

## Oracle

- [ ] A new `run_store.rs` unit test: `persist_report` writes a run to a fresh
  DB, the connection is dropped, the same path is reopened via
  `persist_report` again with a second run, and both runs are still queryable
  via `list_runs` — proving `init_schema`'s `CREATE TABLE IF NOT EXISTS`-style
  init (confirm actual statement) is idempotent and does not clobber existing
  rows.
- [ ] A short `SKILL.md` note under the existing runs-ledger section: the DB
  is a single file under gitignored `runs/local/`, how to back it up (file
  copy while no writer is active, or the project's established pattern if one
  exists — check whether Canary's litestream pattern referenced in
  `~/.factory-lanes/groom/_decisions.md` is meant to apply here; if it's an
  operator-scoped infra decision, say so explicitly and scope this ticket to
  documentation only, not a Litestream integration).
- [ ] `cargo test --all` and the doc build (`RUSTDOCFLAGS="-D warnings" cargo
  doc --no-deps`) pass.

## Notes

Live-code-verified 2026-07-01: `backlog.d/011-runs-database.md`'s children
list still shows item 5 ("Backup/restore note and migration tests")
unchecked while children 1-3 are done. Do NOT stand up Litestream or any new
infra tonight — `_decisions.md`'s crucible section lists "runs database:
SQLite (canary pattern)" as a P0/child-2 concern for the *epic*, but adding
real backup infrastructure is an operator-scoped infra decision (see
bitterblossom epic 2 in the same decisions file, which explicitly owns
Litestream rollout). Scope this ticket to the test + a documentation note
only.

**Why:** directly closes a named, unchecked child of a live epic (`011`),
using only test code and docs — no new infra, no design call.
