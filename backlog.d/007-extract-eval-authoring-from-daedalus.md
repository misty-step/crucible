# Extract eval-authoring from Threshold into Crucible (strategic migration)

Priority: P2 · Status: pending · Estimate: XL (epic)

## Goal

Realize the rechartered boundary: migrate eval/benchmark authoring — arena and
task definitions, fixture corpora, scoring design, adjudication — from Threshold
into Crucible, leaving Threshold focused on Karpathy-style config-optimization
loops that consume Crucible evals.

## Oracle

- [ ] A code-review benchmark is authored, calibrated, and versioned in Crucible
  and consumed by Threshold's optimization loop without Threshold owning its
  definition or scoring design.
- [ ] The arena/taskspec/corpus/adjudication machinery for ≥1 family lives in
  Crucible; Threshold reads it via the Harbor contract; no eval-definition logic
  is duplicated across the two repos.
- [ ] A documented, ratified Crucible ↔ Threshold ownership contract names exactly
  what each repo owns.

## Children (ordered)

1. Map what eval-authoring currently lives in Threshold (arenas, specs, scoring,
   holdout/contamination, Harbor) and classify migrate vs. stay.
2. Agree the Crucible ↔ Threshold ownership contract (operator + Threshold
   governance).
3. Migrate one family (code-review) end to end; Threshold consumes via Harbor.
4. Move corpus/holdout/contamination governance into Crucible.
5. Narrow Threshold to the optimization loop; delete the migrated machinery there.

## Notes

Operator direction (/groom 2026-06-29): "ultimately extract most of that from
Threshold." Large, cross-repo, sequenced AFTER the wedge (002) proves the model.
Needs Threshold-side coordination — do NOT unilaterally edit Threshold. This is the
"durable eval organ" ambition made concrete; it is the biggest bet in the
backlog and the reason Crucible is a separate repo rather than a script.

**Governance update 2026-06-29:** operator ratified that Crucible may author
arena versions / adjudications, clearing the authoring-rights blocker. The actual
extraction still needs a Threshold-side ownership handshake before any cross-repo
change.

**Update 2026-06-30:** child 1 (map) DELIVERED — `docs/daedalus-eval-authoring-map.md`
classifies the Threshold eval-authoring surfaces MIGRATE / STAY / SHARED. The
2026-06-30 review also hardened the contract knowledge that anchors this migration:
Threshold's scorer reads `tests/expected.json` (span `defects[]`), and Crucible now
authors it (002.5) with a verified re-score round-trip. The migration itself still
needs the Threshold governance handshake before any cross-repo change.

Naming: **Threshold** (formerly Daedalus) has not physically renamed on disk, so
the repo directory, its crates, the `daedalus-score` binary, and
`docs/daedalus-eval-authoring-map.md` keep the `daedalus` name until the sibling
repo renames; every such reference in this ticket is real and unchanged.
