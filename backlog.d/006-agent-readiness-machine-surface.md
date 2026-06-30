# Agent-readiness and machine surface

Priority: P2 · Status: pending · Estimate: M (epic)

## Goal

Make Crucible operable and safe for cold agents and the constellation: a
verification skill with real commands, a gate that fails for real errors, a
secret/content-leak gate, stable CLI JSON, and Harbor-schema-validated exports.

## Oracle

- [ ] A cold agent, given only the repo, authors → runs → grades → adjudicates →
  exports one code-review eval via documented commands (the verification
  `SKILL.md` is the acceptance oracle).
- [ ] The repo gate (replacing `test -f VISION.md`) fails the diff on
  build/test/lint breakage, a schema-invalid Harbor export, AND a planted
  secret / proprietary-code leak in a run record or report.
- [ ] The Crucible CLI emits stable JSON + exit codes so Cerberus/Daedalus branch
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
4. Verification `SKILL.md` — author → run → grade → adjudicate → export with the
   real commands; serves as the agent-usability acceptance oracle for child 1.
5. `AGENTS.md` refresh — name the borrowed surfaces + the Rust/TS boundary, link
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
