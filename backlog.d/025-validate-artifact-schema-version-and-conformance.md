# Validate review-artifact schema_version and add a conformance test

Priority: P1 · Status: done · Estimate: S

## Goal
Crucible rejects review artifacts with an unexpected `schema_version` loudly instead of silently dropping unknown fields, and a conformance test binds its hand-mirrored structs to cerberus's canonical fixture.

## Oracle
- [x] Deserialization in `crucible-core/src/artifact.rs` checks `schema_version` and returns a descriptive error on mismatch (today `artifact.rs:176-188` silently drops unknowns).
- [x] A test deserializes the pinned cerberus fixture (`crucible-core/tests/fixtures/cerberus-artifact.json`) through `to_key_finding` (`crucible-core/src/adapter.rs:53-63`) and asserts the mapped output.
- [x] The fixture header/README notes its regeneration source in the cerberus repo (pairs with cerberus refill ticket 035).

## Notes
No shape changes — this makes the existing mirror fail loudly instead of silently when cerberus evolves. Overnight-safe: additive validation + tests.
**Why:** 2026-07-01 composition seam audit, Seam 1 — "cerberus renames a required field → runtime serde parse error caught by no test; optional field → silent drop."

**Progress 2026-07-02 (overnight):** all three oracle bullets landed/confirmed.

`ReviewArtifact.schema_version` now uses `deserialize_with =
deserialize_review_artifact_schema`, reusing the exact `expect_schema` helper
every other Crucible-minted schema-stamped artifact (`EvaluationCard`,
`Label`, `CalibrationRecord`, `JudgmentQueue`, `EvalSpec`) already uses —
consistent idiom, not a new one. Unlike those types, `ReviewArtifact` gets no
`#[serde(default = ...)]` fallback: Crucible only *reads* this envelope
(never mints one), so an absent `schema_version` is as much a mismatch as a
wrong one, not something to default forward. New tests:
`artifact_rejects_an_unexpected_schema_version` and
`artifact_rejects_a_missing_schema_version`; the pre-existing
`artifact_ignores_unknown_envelope_fields` test's fixture JSON was carrying
`"schema_version": "v1"` as a stand-in value — updated to the real
`cerberus.review_artifact.v1` constant (now exported as
`REVIEW_ARTIFACT_SCHEMA`) since a placeholder value would now correctly fail.
Verified no regression: every committed real artifact fixture across both
crates already carries the correct schema_version — `cargo test --all`
green with zero fixture updates needed.

The second and third oracle bullets turned out to already be met by existing
tests, found on inspection rather than needing new code:
`crucible-core/tests/adapter_map.rs`'s `real_artifact_maps_to_well_formed_key_findings`
already deserializes the pinned fixture and projects it through
`to_key_findings` (the public plural entry point; `to_key_finding` singular is
a private helper), asserting file/line/category/severity/description on the
mapped output — this is the "conformance test" the oracle asks for, just
already existing under a different filename than the ticket's line-number
citation implied. The fixture's regeneration source (`cerberus/evidence/
self-review-001/artifact.json`) is already documented in both consuming test
files' module doc comments (`fixture_parse.rs`, `adapter_map.rs`) rather than
a separate fixture-directory README; that's the established pattern for every
other fixture in this repo, so no new README was added.
