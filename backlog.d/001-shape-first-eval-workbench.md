# Shape the first Crucible eval workbench slice

Priority: P1
Status: done
Estimate: M

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
- Smallest implementation stack and gate: borrow execution (Daedalus arenas +
  Cerberus); Crucible owns the eval artifact + calibration + export. Gate work in
  epic `006-agent-readiness-machine-surface`.
- Crucible vs Daedalus vs project repos: rechartered in `VISION.md` — Crucible
  owns eval authoring/calibration; Daedalus optimizes against trusted evals; the
  migration is epic `007-extract-eval-authoring-from-daedalus`.

## Disposition

Superseded by epics 002–007. Proposed for archive to `backlog.d/_done/`
(awaiting operator ratification — groom does not auto-archive).
