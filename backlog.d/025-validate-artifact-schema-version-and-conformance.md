# Validate review-artifact schema_version and add a conformance test

Priority: P1 · Status: ready · Estimate: S

## Goal
Crucible rejects review artifacts with an unexpected `schema_version` loudly instead of silently dropping unknown fields, and a conformance test binds its hand-mirrored structs to cerberus's canonical fixture.

## Oracle
- [ ] Deserialization in `crucible-core/src/artifact.rs` checks `schema_version` and returns a descriptive error on mismatch (today `artifact.rs:176-188` silently drops unknowns).
- [ ] A test deserializes the pinned cerberus fixture (`crucible-core/tests/fixtures/cerberus-artifact.json`) through `to_key_finding` (`crucible-core/src/adapter.rs:53-63`) and asserts the mapped output.
- [ ] The fixture header/README notes its regeneration source in the cerberus repo (pairs with cerberus refill ticket 035).

## Notes
No shape changes — this makes the existing mirror fail loudly instead of silently when cerberus evolves. Overnight-safe: additive validation + tests.
**Why:** 2026-07-01 composition seam audit, Seam 1 — "cerberus renames a required field → runtime serde parse error caught by no test; optional field → silent drop."
