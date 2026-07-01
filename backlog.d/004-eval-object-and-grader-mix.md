# The eval object and per-eval grader-mix model

Priority: P1 · Status: in-progress · Estimate: L (epic)

## Goal

Define the durable, Crucible-owned eval/benchmark artifact: a declarative spec
that names its own grader mix (deterministic + agentic/model-judge + human) plus
the record/label/calibration types, with Harbor as the export contract — not a
reinvented one.

## Oracle

- [ ] An eval is defined in one declarative spec naming task, inputs/outputs,
  fixtures (by hash), grader manifest (per grader: deterministic | agentic |
  human), baselines, aggregation, uncertainty rules, and the decision it informs.
- [ ] The same spec drives both a near-deterministic eval and a
  human-judgment-heavy eval (the per-eval spectrum) with no change to core code.
- [ ] Run records, labels (append-only), and calibration records share one serde
  schema with `schema_version`; export validates against the Harbor
  task-directory format and round-trips into Threshold (golden-fixture test).

## Children (ordered)

1. Core types contract — `EvalSpec`, `FixtureRef(hash)`,
   `GraderManifest{deterministic|agentic|human}`, `RunRecord`, `Label`,
   `CalibrationRecord`, `Aggregate{score, CI, paired-delta}`, `Provenance`,
   `schema_version`.
2. Spec validate + grader classification + deterministic-vs-human cost report
   (expanded in `014-agent-first-surfaces-and-honest-specs.md`).
3. Store — moved to `011-runs-database.md`; queue remains a view over persisted
   run/label records, not a third store.
4. ✅ Harbor export/import + golden-fixture round-trip test — delivered via 002.5.

## Notes

Architecture lane: the type contract is the narrow waist; artifacts ARE the API
(no daemon/RPC/auth). Do NOT build a storage-backend abstraction or a grader
plugin registry (three enumerated kinds). Extract this from what the wedge (002)
actually needed — do not front-run it. The phone UI (005) and Threshold both read
these artifacts, so getting the queue + label + export schemas right here means
the UI adds zero new core design.

**Update 2026-06-30:** child 1 (type contract) DELIVERED — `EvalSpec` +
`GraderManifest{deterministic|agentic|human}` + `FixtureRef(hash)`, `Label`,
`CalibrationRecord`, `Provenance`/`EvaluationCard`, `Aggregate` — all serde
`schema_version`-stamped with round-trip tests and a light version-validation guard
that rejects an unknown version rather than silently treating it as v1. Child 3
(SQLite index + content-addressed blob store) deliberately NOT built — premature;
artifacts-on-disk suffice for the wedge (per this epic's own non-goal note). Child 4
(Harbor export + golden round-trip) landed via 002.5.

**Factory groom 2026-07-01:** the type contract survives, but decorative fields
must stop lying. Specs must either wire declared graders, baselines, fixtures,
and confidence into execution or validation must refuse them until they are real
(`014`).
