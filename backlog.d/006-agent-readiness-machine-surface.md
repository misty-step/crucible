# Agent-readiness and machine surface

Priority: P1 · Status: in-progress · Estimate: M (epic)

## Goal

Make Crucible operable and safe for cold agents and the constellation: a
verification skill with real commands, a gate that fails for real errors, a
secret/content-leak gate, stable CLI JSON, and Harbor-schema-validated exports.

## Oracle

- [x] A cold agent, given only the repo, runs → grades → adjudicates → exports
  one code-review eval via documented commands (the verification `SKILL.md` is
  the acceptance oracle for the current wedge).
- [ ] A cold agent authors and runs a new benchmark through CLI and MCP without
  Threshold-only knowledge (covered by 010/014).
- [ ] The repo gate (replacing `test -f VISION.md`) fails the diff on
  build/test/lint breakage, a schema-invalid Harbor export, AND a planted
  secret / proprietary-code leak in a run record or report.
- [ ] The Crucible CLI emits stable JSON + exit codes so Cerberus/Threshold branch
  on verdicts headlessly; run artifacts that embed real diffs are
  gitignored/redacted, not committed raw.

## Children (ordered)

1. CLI JSON contract + stable exit codes.
2. Real repo gate — Rust fmt/clippy/test/build (+ TS typecheck/lint/build when
   the UI lands) + Harbor export-schema validation. Gate-the-diff, ratcheted.
3. Secret/content-leak gate — gitleaks/trufflehog-class scan + a content policy
   (raw model outputs / diffs redacted or allowlisted) on the diff AND before any
   report/export is published. Close the `runs/*` gitignore gap for wherever
   Crucible writes artifacts (today `.gitignore` ignores only `runs/*/artifacts/`).
4. ✅ Verification `SKILL.md` — author → run → grade → adjudicate → export with
   the real commands; serves as the agent-usability acceptance oracle for child 1.
5. ✅ `AGENTS.md` refresh — name the borrowed surfaces + the Rust/TS boundary, link
   the skill, document the CLI/export/security contracts.

## Notes

Agent-readiness lane. The export contract is Harbor (epic 004), not a new schema.
Security surface is real: eval runs invoke models with API keys and store outputs
over real diffs that can embed proprietary code. Defer the TS half of the gate
until the UI (005) lands, but specify it now so it is not bolted on weakly later.

**Update 2026-06-29:** child 2 (real repo gate) DELIVERED — `scripts/check.sh`
(fmt / clippy `-D warnings` / test / build), AGENTS.md gate section updated, gates
the diff. CLI `--json` machine surface (child 1) partially landed via `crucible
adapt`/`grade`. Still pending: the **secret/content-leak scan** (child 3) —
thermonuclear review flagged that committed test fixtures vendor real Cerberus
review content (no live leak today; defense-in-depth before the eval surface
persists real diffs).

**Update 2026-06-30:** child 1 (CLI JSON + stable exit codes 0/1/2, `schema_version`
on every `--json`) and child 3 (secret/content-leak scan) DELIVERED. The scan is
portable — review caught a real macOS bash-3.2 `mapfile` no-op that silently
disabled the floor on the author's own shell — covers AWS/Stripe/GCP/JWT/URL-cred
families, warns when gitleaks is absent (no false "clean"), and `runs/` is now fully
gitignored; AGENTS.md content policy corrected to credential scope + the cargo-doc
`-D warnings` step was added to the gate. Still pending: child 4 (verification
`SKILL.md` — author→run→grade→adjudicate→export with the now-real commands).

**Update 2026-07-01:** child 4 (repo-local verification `SKILL.md`) DELIVERED
for the current code-review wedge, including declared-spec runs, Cerberus
receipt-bundle handoff shape, headless grade/adjudicate/export commands, and
the static adjudication-panel path. Agent/Threshold access also landed as
`crucible mcp`, exposing the shared `crucible run` path as the `crucible_run`
stdio MCP tool; the integration test initializes MCP, lists the tool, invokes a
declared spec, and verifies the Wilson-scored `crucible.run_report.v1` plus
written `run-report.json`. Still pending: child 5 (`AGENTS.md` refresh) and
Harbor export-schema validation ratcheting when the schema check becomes a
repo-owned gate.

**Factory groom 2026-07-01:** agent-first means CLI + MCP for define/manage/run,
not just one `crucible_run` tool. The broader surface is tracked in `014`; keep
this epic as the gate/contract/operability spine.

**Progress 2026-07-02 (overnight):** child 5 landed. `AGENTS.md`'s "Current
state" bullet was badly stale — it still named the author-and-run engine
(`010`) as the *next* priority when 010, the runs database (`011`), the
agentic judge tier (`012`), the one-scorer consolidation (`013`), the first
real Cerberus benchmark (`015`), the adjudication writeback loop (`005`
partial), and spec honesty/`validate` (`014` partial) had all already landed.
Rewrote it to name what is actually real today (three runner kinds, the SQLite
ledger, `validate`, the judge tier's calibration/canary guard, the writeback
server) and where the remaining open work lives, so a cold agent reading
`AGENTS.md` first gets the accurate picture instead of a nine-day-old one.
`SKILL.md` (the command contract `AGENTS.md` points to) got the matching
update: `crucible validate`, the agentic-judge and cerberus-review-quality run
examples, the judge-gaming canary and calibration-record behavior,
`adjudication-panel --serve`'s real writeback loop, and `crucible_validate` in
the MCP tool list — all commands re-verified live against the real binary
before committing. Docs-only; no code changed, gate still green.
