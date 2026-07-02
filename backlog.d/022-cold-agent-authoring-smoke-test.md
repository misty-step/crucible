# Cold-agent fixture-backed benchmark-authoring smoke test

Priority: P1 · Status: ready · Estimate: M

## Goal

Close backlog `014`'s remaining child 5: "Fixture-backed authoring smoke: a
cold agent creates a tiny benchmark that validates and runs hermetically."
The epic's own 2026-07-02 progress note is explicit that every spec merged so
far was authored by the same lane across prior sessions — no one has proven a
genuinely fresh reader (only `SKILL.md` + the schema, no chat context) can
author a spec that passes `crucible validate` and `crucible run` end to end
with zero sibling-repo dependencies.

## Oracle

- [ ] A new fixture-only prompt-benchmark spec (task + rubric, no
  `daedalus_trials`/sibling-repo corpus reference) is authored using *only*
  `SKILL.md`'s documented fields and the schema Rust types as the reference —
  do not copy an existing committed spec's exact shape verbatim; treat it as
  a genuine authoring exercise and note any place `SKILL.md` under-specified
  a field or a required-but-undocumented default.
- [ ] The new spec lives under `crucible/tests/fixtures/specs/` (matching the
  existing fixture convention) and is exercised by a `crucible/tests/cli.rs`
  test: `crucible validate <spec> --json` reports `valid: true`, then
  `crucible run <spec> --out <tmp> --json` runs hermetically (no
  `OPENROUTER_API_KEY`, no sibling checkout) and exits 0.
- [ ] If any `SKILL.md` gap is found during authoring (a field whose
  behavior wasn't documented, or a validation error whose message didn't
  explain the fix), file it as a doc fix in the same PR or as a follow-up
  ticket — this is exactly the signal this child is meant to surface.
- [ ] `cargo test --all` passes; the new fixture spec requires no network
  access to validate or run.

## Notes

Live-code-verified 2026-07-01: `backlog.d/014-agent-first-surfaces-and-
honest-specs.md`'s Notes section states children 1-4 landed but child 5
("Remaining: child 5 (a genuinely fresh/cold-agent authoring smoke test —
tonight's specs were all authored by this same lane across prior sessions,
not demonstrated cold)") is still open. `evals/prompt-smoke-v0.json` is the
only fully-hermetic prompt-benchmark spec today, and it was authored by the
same lane. This ticket is overnight-safe specifically because the deliverable
is a test artifact plus documentation gap-filling, not a product-surface
design decision — the "cold agent" framing is a verification stance to adopt
while writing the fixture, not new scope.

**Why:** directly named as the sole open child of a live P1 epic, in the
exact "spec-authoring ergonomics with existing patterns" bucket OVERNIGHT.md
calls out for crucible tonight.
