# Investigate folding Threshold's config-search into Crucible as a feature

Priority: P2 · Status: pending · Estimate: M (investigation + decision, not implementation)

## Goal

The operator is considering folding Threshold (`~/Development/daedalus`,
narrative-renamed, on-disk name unchanged) — specifically its config-space
search over benchmarks — into Crucible as a feature rather than keeping it a
sibling repo. Produce an honest decision, not a default toward consolidation.
This ticket is investigation + decision; if the answer is "fold in," the fold
itself is a separate, later epic.

## Why this is not a small call

VISION.md already drew this boundary deliberately and recently:

- "Not an optimizer over agent configurations. Threshold owns that; Crucible
  designs the measurement Threshold optimizes against." (`What This Is Not`)
- "If the question is 'what should the measurement surface be, and do we
  trust it?', that is Crucible. If the question is 'which harness/agent
  configuration scores highest against this trusted measurement surface?',
  that is Threshold." (`The Role In The Constellation`)
- Operator clarification, 2026-06-29 (`/groom`): "keep Crucible as a
  separate, rechartered repo" (from a prior monolith). That decision was
  about splitting eval-authoring *out* of Threshold into Crucible, not about
  Threshold's search loop moving *in* — but a Threshold fold-in reopens the
  same repo-boundary question from the other direction one week later.

Folding the search loop in is not obviously wrong, but it is not free: it
either contradicts the measurement/optimization split VISION.md just wrote
down, or it requires the split to be re-articulated as "Crucible owns both,
but the boundary between measurement code and search code stays inside one
repo" — that distinction needs to be made explicit either way, not left
implicit.

## Oracle

- [ ] A decision doc (this ticket's Notes, or a `docs/` file it points to)
  states one of: **fold in** (Threshold's search/Lab-stage code becomes a
  Crucible crate/feature), **stay separate** (tighten the Harbor-contract
  boundary instead, see `007`), or **partial** (e.g. share a stats/measure
  crate without merging the search loop or the six-stage pipeline) — with the
  operator's explicit sign-off, not an agent's inference.
- [ ] The doc names, concretely, what "Threshold" means in this decision:
  Threshold owns a six-stage pipeline (Specify → Lab → Contract → Deploy →
  Observe → Reiterate per `daedalus/DESIGN.md`) — "fold in threshold" almost
  certainly means the Lab (search) stage, not the whole pipeline. State that
  scope boundary explicitly before deciding; do not let "threshold" quietly
  mean "the whole daedalus repo."
- [ ] Investigation quantifies actual code overlap today: does
  `crucible-core::measure` duplicate anything in daedalus's scoring/stats
  crates (bootstrap CI, McNemar, noise-floor kernels)? Cite file paths and
  crate names on both sides, not an estimate.
- [ ] Tradeoffs section covers, at minimum: (a) the VISION.md
  measurement-vs-optimization boundary and whether folding contradicts it;
  (b) blast-radius cost — Crucible is currently a small, coherent tool
  (validate/run/grade/adjudicate/export + MCP); absorbing a six-stage
  pipeline (candidate generation, Pareto archive, launch contracts, deploy/
  observe/reiterate) is a large scope increase against the "deep module,
  small surface" standard, even if scoped to just the Lab stage; (c) the
  Harbor-contract coupling cost of staying separate — how much sync overhead
  does the current two-repo setup actually impose, concretely (cite `007`'s
  migration-map status); (d) reversibility — which choice is cheaper to
  reverse if wrong.
- [ ] If the decision is "fold in": open a new, separate epic ticket scoping
  the migration (do not implement it as a child of this ticket). If "stay
  separate" or "partial": link the decision back into `007`'s ownership
  contract so the two tickets do not contradict each other.

## Boundaries

No code changes here, and no edits to `~/Development/daedalus` or exocortex —
this ticket is Crucible-side investigation and decision-recording only. Any
cross-repo change from a "fold in" decision needs Threshold-side coordination,
same as `007`.

## Notes

Report cross-reference: `ai-evals-benchmarks-report.md` does not address
build-vs-buy for an internal optimizer/search sibling repo (§8's "Build vs.
integrate" is about commodity eval tooling, not this); this ticket is
operator-originated, not report-derived. Filed as part of the 2026-07-02
eval-OS intake pass alongside `026`, `027`, `029`, `030`.
