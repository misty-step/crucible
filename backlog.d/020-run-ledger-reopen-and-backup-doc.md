# Runs ledger: reopen-safety test + backup/restore doc note

Priority: P2 · Status: done · Estimate: S

## Goal

Close backlog `011`'s remaining child 5 ("Backup/restore note and migration
tests"): prove the SQLite run ledger (`crucible/src/run_store.rs`,
`open_initialized`/`init_schema`, lines 586-703) survives being reopened
against an existing populated database file without data loss or schema
error, and document the backup story in `SKILL.md` (the file already
documents `--db <PATH>` overrides but says nothing about backing up or
restoring `runs/local/crucible-runs.sqlite`).

## Oracle

- [x] A new `run_store.rs` unit test: `persist_report` writes a run to a fresh
  DB, the connection is dropped, the same path is reopened via
  `persist_report` again with a second run, and both runs are still queryable
  via `list_runs` — proving `init_schema`'s `CREATE TABLE IF NOT EXISTS`-style
  init (confirm actual statement) is idempotent and does not clobber existing
  rows.
- [x] A short `SKILL.md` note under the existing runs-ledger section: the DB
  is a single file under gitignored `runs/local/`, how to back it up (file
  copy while no writer is active, or the project's established pattern if one
  exists — check whether Canary's litestream pattern referenced in
  `~/.factory-lanes/groom/_decisions.md` is meant to apply here; if it's an
  operator-scoped infra decision, say so explicitly and scope this ticket to
  documentation only, not a Litestream integration).
- [x] `cargo test --all` and the doc build (`RUSTDOCFLAGS="-D warnings" cargo
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

**Progress 2026-07-02 (overnight):** landed, test + docs only, no new
infrastructure. `persist_report_reopens_an_existing_populated_db_without_data_loss`
persists a first run, then persists a *second* run against the same DB
path — since `persist_report` opens its own `Connection` per call via
`open_initialized`, this is the real reopen scenario a second `crucible run`
invocation hits in practice, not a simulated one. Asserts both runs are
listed and both are independently `show_run`-able (a reopen that silently
corrupted one run's detail rows while leaving its summary row intact would
slip past a count-only assertion). `SKILL.md` gained a "Backup/restore"
paragraph: the ledger is one gitignored file, copy it while no writer holds
the connection (SQLite gives no consistent-snapshot guarantee mid-write), and
restoring is just replacing the file — `init_schema`'s `CREATE TABLE IF NOT
EXISTS` statements (confirmed exact wording in `run_store.rs`) mean the next
`crucible run` reopens a restored file with no migration step. Explicitly
scoped to documentation, per the ticket's own instruction: real automated
backup (Canary's Litestream pattern) stays an operator-scoped infra decision,
not stood up here.
