# Keep Crucible publicable and integrated with the factory fleet

Priority: P2 · Status: ready · Estimate: M (epic)

## Goal

Remove stale repo-contract prose and local-machine assumptions that make
Crucible harder for cold agents, public readers, or the factory fleet to use.

## Oracle

- [x] `AGENTS.md`, README, SKILL, and VISION agree on the current state: Rust
  code exists, Crucible owns selected execution, and Threshold is the optimizer.
- [ ] The flagship spec no longer hardcodes `../../daedalus` paths without an
  explicit local-only marker or a portable fixture alternative.
- [ ] Reclaimed go-council residue is closed or labeled so it cannot be mistaken
  for current Crucible roadmap work.
- [x] Publicable policy holds: MIT license present, no instance data, personal
  paths, tailnet names, or raw run records in tracked files except deliberate
  fixture paths.
- [ ] Landmark release intelligence remains wired and documented.

## Verification System

- Claim: a cold factory agent can understand and run Crucible without stale
  contract traps or personal-instance leakage.
- Falsifier: repo docs still say there is no application code, the default spec
  fails only because a sibling path is assumed, or tracked files reveal instance
  names/raw run data.
- Driver: `rg` hygiene checks, `crucible validate`, and the repo gate.
- Grader: no stale-string hits for the known traps plus successful fixture-run
  commands from SKILL.
- Evidence packet: command transcript and PR diff.
- Cadence: every groom or release-integration change.

## Children

1. ✅ Refresh stale `AGENTS.md` current-state line.
2. Fix or mark local-only hardcoded Daedalus/Threshold paths in the flagship spec
   and docs. Personal absolute paths are removed; the flagship spec portability
   marker remains.
3. Close or label GitHub issue #15 as archived go-council residue.
4. ✅ Keep MIT license and repository metadata aligned to `misty-step/crucible`.
5. Add a small hygiene check if these traps recur.

## Notes

The 2026-07-01 groom verified issue #15 is still open and belongs to the old
go-council project. Do not close it silently from a local docs PR if GitHub
project policy wants labels instead; the backlog item makes the disposition
explicit.

**Factory groom 2026-07-01:** MIT license, repository metadata, stale AGENTS
state, README/SKILL personal absolute paths, and the authoring-map absolute path
were fixed in the groom PR. Remaining: mark or eliminate the flagship spec's
local-only sibling path and dispose of GitHub issue #15.
