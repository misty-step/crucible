# Crucible

Crucible is the eval and benchmark workbench for Misty Step's AI and agent work.

It is where evals and benchmarks are brainstormed, defined, designed,
implemented, calibrated, and iterated: deterministic checks where possible,
agentic/model judges where useful (calibrated before trusted), human judgment
where needed, and clear uncertainty around every result. Daedalus consumes
Crucible's trusted evals to optimize harness and agent configurations.

Its one principle: **refuse to report a delta it cannot defend** — every rate
carries an interval, every judge a calibration, every comparison a noise-floor
check.

For the project north star and the boundary with Daedalus and Harness Kit, read
[`VISION.md`](VISION.md).

## Current State

Docs-first seed repo; no application code yet. The first implementation is shaped:
the agentic **code-review eval wedge** (`backlog.d/002-codereview-eval-wedge.md`)
— industrialize Daedalus's manual adjudication, calibrate the judge, bootstrap
labels for real diffs, and emit Harbor benchmark tasks Daedalus re-scores.

## Backlog

- `001` — shaping (done, completed by /groom 2026-06-29)
- `002` — code-review eval wedge (ready; the first pickup)
- `003` — measurement rigor core (the trust machinery / moat)
- `004` — eval object and per-eval grader-mix model
- `005` — phone-first adjudication queue
- `006` — agent-readiness and machine surface
- `007` — extract eval-authoring from Daedalus (strategic migration)

## Gate

```sh
test -f VISION.md
rg -n "VISION\\.md" AGENTS.md README.md
```
