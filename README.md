# Crucible

Crucible is the eval workbench for Misty Step's AI and agent work.

It exists to help design, run, judge, report, and iterate evals: deterministic
checks where possible, model judges where useful, human judgment where needed,
and clear uncertainty around every result.

For the project north star and the boundary with Daedalus and Harness Kit, read
[`VISION.md`](VISION.md).

## Current State

This is a docs-first seed repo. The first implementation should be shaped around
one concrete eval family and one real judgment workflow, not a generic platform
sketch.

## Early Questions

- What is the first eval family?
- What does the human-judgment queue need to feel good on a phone?
- What export shape should Daedalus and Harness Kit consume?
- Which code belongs in this repo versus project-local eval directories?

## Gate

```sh
test -f VISION.md
rg -n "VISION\\.md" AGENTS.md README.md
```
