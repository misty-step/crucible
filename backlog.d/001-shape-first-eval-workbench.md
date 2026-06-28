# Shape the first Crucible eval workbench slice

Priority: P1
Status: ready-for-shaping
Estimate: M

## Goal

Choose one real eval family and shape the first Crucible implementation around
it: eval definition, run inputs, grader mix, human-judgment workflow, result
report, and export target.

## Why

Crucible should not start as a generic eval platform. It should prove one useful
loop end to end, then generalize from the parts that survive contact with real
use.

Good first candidates:

- Harness Kit primitive evals: raw agent vs Harness Kit vs alternative
  primitive.
- Daedalus eval packages: task/eval surfaces Daedalus can optimize agent
  configurations against.
- Product behavior evals for Memory Engine, Allie, or review agents.

## Oracle

- The shaped packet names the first eval family and why it is first.
- It defines the eval object: task, inputs, outputs, fixtures, grader mix,
  human judgment, baselines, aggregation, uncertainty, and export target.
- It sketches the human-judgment UI, including the phone review path.
- It names the smallest implementation stack and gate.
- It explicitly separates Crucible responsibilities from Daedalus and project
  repos.

## Verification

- Review against `VISION.md`.
- Confirm the chosen first slice has a real downstream consumer.
- Confirm the plan does not require building a broad platform before one eval
  loop works.
