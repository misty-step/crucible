# Cold-agent fixture-backed benchmark-authoring smoke test

Priority: P1 · Status: done · Estimate: M

## Goal

Close backlog `014`'s remaining child 5: "Fixture-backed authoring smoke: a
cold agent creates a tiny benchmark that validates and runs hermetically."
The epic's own 2026-07-02 progress note is explicit that every spec merged so
far was authored by the same lane across prior sessions — no one has proven a
genuinely fresh reader (only `SKILL.md` + the schema, no chat context) can
author a spec that passes `crucible validate` and `crucible run` end to end
with zero sibling-repo dependencies.

## Oracle

- [x] A new fixture-only spec (no `daedalus_trials`/sibling-repo corpus
  reference) is authored using *only* `SKILL.md`'s documented fields and the
  schema Rust types as the reference — do not copy an existing committed
  spec's exact shape verbatim; treat it as a genuine authoring exercise and
  note any place `SKILL.md` under-specified a field or a
  required-but-undocumented default. (Runner family changed from
  `prompt_benchmark` to `key_recall`/`cerberus_receipt_bundles` — see
  progress note; the ticket's own two constraints, "hermetic" and
  "prompt-benchmark," are mutually exclusive with the current runners.)
- [x] The new spec lives under `crucible/tests/fixtures/specs/` (matching the
  existing fixture convention) and is exercised by a `crucible/tests/cli.rs`
  test: `crucible validate <spec> --json` reports `valid: true`, then
  `crucible run <spec> --out <tmp> --json` runs hermetically (no
  `OPENROUTER_API_KEY`, no sibling checkout) and exits 0.
- [x] If any `SKILL.md` gap is found during authoring (a field whose
  behavior wasn't documented, or a validation error whose message didn't
  explain the fix), file it as a doc fix in the same PR or as a follow-up
  ticket — this is exactly the signal this child is meant to surface.
- [x] `cargo test --all` passes; the new fixture spec requires no network
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

**Progress 2026-07-02 (overnight):** landed, with one correction to the
ticket's own suggested shape. The ticket asked for a "fixture-only
prompt-benchmark spec... requires no network access to validate or run" —
but `prompt_benchmark` always makes a live OpenRouter call
(`OpenRouterClient::from_credential_env` unconditionally builds a real HTTP
client); there is no hermetic mode for that runner family. Those two
constraints ("prompt-benchmark" and "hermetic") are mutually exclusive with
the current runner architecture, so the fixture was authored against
`key_recall`/`cerberus_receipt_bundles` instead — the runner family that
*is* genuinely hermetic (deterministic key-matching over committed JSON,
zero network). This is a factual correction to a ticket whose own two
oracle constraints conflicted, not a scope or taste substitution.

New fixtures, deliberately fresh content (not copied from
`cerberus-receipt-fixture.json`): `tests/fixtures/cold-agent-artifact.json`
(a `retry loop stops one attempt early` finding, distinct file/line/category
from the existing self-review fixture), `tests/fixtures/
cold-agent-receipt-bundle.json`, `tests/fixtures/cold-agent-expected.json`
(two seeded defects — one matched, one deliberately missed, so the run
proves a non-trivial recall < 100%, not a degenerate 100%-or-0% case), and
`tests/fixtures/specs/cold-agent-smoke-v0.json` tying them together. Live
end-to-end proof, not just the test: `crucible validate` reports `valid:
true, runnable: true`, and `crucible run` with `OPENROUTER_API_KEY` unset
exits 0 with `successes: 1, n: 2` — exactly as designed.

`SKILL.md` gap found and fixed in the same PR: the existing
`cerberus_receipt_bundles` section named the required fields
(`artifact`/`receipt_bundle`/`expected`) but never documented the internal
shape those referenced files must carry — a cold author has to read
`spec_run.rs`'s `validate_cerberus_receipt`/`receipt_artifact_uri_matches`
Rust source to learn that `receipt_bundle.validation.status` must be exactly
`"passed"` and `receipt_bundle.artifact_uri` must match the spec's own
`task.artifact` string. Added a short paragraph documenting both
constraints, plus the hermetic-vs-network runner-family distinction that
motivated this ticket's correction.
