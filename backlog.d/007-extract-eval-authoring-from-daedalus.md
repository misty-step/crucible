# Extract eval-authoring from Daedalus into Crucible (strategic migration)

Priority: P2 · Status: pending · Estimate: XL (epic)

## Goal

Realize the rechartered boundary: migrate eval/benchmark authoring — arena and
task definitions, fixture corpora, scoring design, adjudication — from Daedalus
into Crucible, leaving Daedalus focused on Karpathy-style config-optimization
loops that consume Crucible evals.

## Oracle

- [ ] A code-review benchmark is authored, calibrated, and versioned in Crucible
  and consumed by Daedalus's optimization loop without Daedalus owning its
  definition or scoring design.
- [ ] The arena/taskspec/corpus/adjudication machinery for ≥1 family lives in
  Crucible; Daedalus reads it via the Harbor contract; no eval-definition logic
  is duplicated across the two repos.
- [ ] A documented, ratified Crucible ↔ Daedalus ownership contract names exactly
  what each repo owns.

## Children (ordered)

1. Map what eval-authoring currently lives in Daedalus (arenas, specs, scoring,
   holdout/contamination, Harbor) and classify migrate vs. stay.
2. Agree the Crucible ↔ Daedalus ownership contract (operator + Daedalus
   governance).
3. Migrate one family (code-review) end to end; Daedalus consumes via Harbor.
4. Move corpus/holdout/contamination governance into Crucible.
5. Narrow Daedalus to the optimization loop; delete the migrated machinery there.

## Notes

Operator direction (/groom 2026-06-29): "ultimately extract most of that from
Daedalus." Large, cross-repo, sequenced AFTER the wedge (002) proves the model.
Needs Daedalus-side coordination — do NOT unilaterally edit Daedalus. This is the
"durable eval organ" ambition made concrete; it is the biggest bet in the
backlog and the reason Crucible is a separate repo rather than a script.

**Governance update 2026-06-29:** operator ratified that Crucible may author
arena versions / adjudications, clearing the authoring-rights blocker. The actual
extraction still needs a Daedalus-side ownership handshake before any cross-repo
change.
