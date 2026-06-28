# Crucible repo contracts

- North star: read `VISION.md` before changing product scope, eval semantics,
  runner boundaries, UI direction, or the Daedalus/Harness Kit relationship.
- Current state: docs-first seed repo. Do not invent an application stack until
  the first concrete eval family and UI workflow are shaped.
- Rust by default for durable runner, storage, scoring, export, and validation
  code. A TypeScript/React web layer is acceptable when the human-judgment UI is
  the work; keep that boundary explicit.
- Evals are measurement systems, not demo scripts. Every eval design must name
  task, inputs, outputs, graders, baselines, human judgment if any, uncertainty,
  and the decision it informs.
- Daedalus owns agent-configuration optimization against trusted evals. Crucible
  owns eval design, run management, judgment collection, reporting, and export.
- Project-specific evals may live in the project repo that cares about them.
  Crucible should help author, run, review, or export them without becoming a
  dumping ground.
- Backlog: active work lives in `backlog.d/NNN-*.md`; closed work moves to
  `backlog.d/_done/`.

## Gate

Until code exists, the repo gate is:

```sh
test -f VISION.md
rg -n "VISION\\.md" AGENTS.md README.md
```

When implementation begins, replace this with the repo-owned build/test/lint
gate and keep this section current.
