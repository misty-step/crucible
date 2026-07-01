# Shape the first Crucible eval workbench slice

Priority: P1 · Status: done · Estimate: M

## Goal

Choose one real eval family and shape the first Crucible implementation around
it.

## Outcome (completed by /groom 2026-06-29)

Shaping is complete. The strategic groom on 2026-06-29 answered every oracle
below and recorded the results in `VISION.md` (rechartered) and the epics it
emitted:

- First eval family + why first: agentic code-review quality — see `VISION.md`
  "Early Shape" and epic `002-codereview-eval-wedge`.
- Eval object (task, inputs, outputs, fixtures, grader mix, human judgment,
  baselines, aggregation, uncertainty, export): epic `004-eval-object-and-grader-mix`.
- Human-judgment UI incl. phone path: epic `005-phone-adjudication-queue`.
- Smallest implementation stack and gate: borrow execution (Threshold arenas +
  Cerberus); Crucible owns the eval artifact + calibration + export. Gate work in
  epic `006-agent-readiness-machine-surface`.
- Crucible vs Threshold vs project repos: rechartered in `VISION.md` — Crucible
  owns eval authoring/calibration; Threshold optimizes against trusted evals; the
  migration is epic `007-extract-eval-authoring-from-daedalus`.

## Disposition

Archived by the 2026-07-01 factory groom. Superseded by the active epics in
`backlog.d/`, especially `010-author-and-run-engine.md`, `011-runs-database.md`,
and `012-three-judge-tiers-real.md`.
