# Crucible Vision

Crucible exists to make evals and benchmarks easier to brainstorm, define,
design, build, run, judge, calibrate, understand, and improve. It is a
top-priority Misty Step lab project because better agent work requires
measurement that survives contact with reality.

Crucible is the eval and benchmark workbench for Misty Step's agent and product
experiments. A good eval is not just a prompt and a score. It is a task
definition, fixture or corpus, execution plan, grader mix, calibration path,
human-judgment workflow, result surface, uncertainty model, and iteration loop
that tells us whether a model, agent, prompt, tool surface, product change, or
workflow is actually getting better.

The product turns "we need an eval for this" into a concrete, auditable
measurement system: what task are we measuring, what counts as good, what can be
checked deterministically, what needs agentic judgment, what needs human
judgment, how confident are we, and what should change next?

## The One Principle

Crucible refuses to report a delta it cannot defend. A rate without an interval,
a judge without calibration, a delta reported inside the noise floor — these are the
failures Crucible exists to prevent. The workbench's job is to make the
measurement trustworthy, then state plainly what it does and does not prove.

## Why This Exists

Misty Step needs real science around AI systems. Agent work is too nonlinear for
vibes: a new model, a different system prompt, a tool allowlist, a subagent, a
reasoning budget, or a UI change can feel better while producing worse outcomes.
Without evals, every improvement claim collapses into taste, demos, and recent
memory.

Crucible is where those claims get a measurement worth trusting. It should make
eval design accessible enough to use often, rigorous enough to trust, and humane
enough that the operator can contribute judgment without turning it into a
miserable chore.

## The Role In The Constellation

Crucible owns the eval and benchmark as a durable artifact — defining,
designing, implementing, running, judging, calibrating, storing, reporting, and
iterating it. **Threshold** (formerly Daedalus) consumes Crucible's trusted evals
and run records to optimize harness and agent configurations. The rename is
narrative only: Threshold's on-disk checkout and its `daedalus-score` binary keep
the `daedalus` name until the sibling repo physically renames.

- Crucible is where evals and benchmarks are brainstormed, defined, designed,
  implemented, run against selected configs, calibrated, and iterated: task
  definitions, corpora and fixtures, grader mix, scoring rules, run records, and
  trust/calibration.
- Threshold runs Karpathy-style auto-research and optimization loops that use
  Crucible's evals to find the harness and agent configuration that masters a
  given measurement surface.
- Harness Kit carries reusable agent primitives and portable eval contracts for
  primitive-level claims.
- Product repos keep project-specific evals close to the behavior they care
  about; Crucible helps author, calibrate, and export them.

If the question is "what should the measurement surface be, and do we trust
it?", that is Crucible. If the question is "which harness/agent configuration
scores highest against this trusted measurement surface?", that is Threshold.

Direction of travel: the eval-authoring machinery that currently lives in
Threshold — arena and task definitions, fixture corpora, scoring design,
adjudication, and run records where appropriate — should migrate into Crucible
over time, leaving Threshold focused on the optimization search loop. Until that
migration lands, Crucible reads and writes the existing Threshold arena and
Harbor artifacts in place rather than duplicating them.

## Form Factor

Ratified 2026-07-09 after a live week of fleet use and an industry survey
(Eve/Vercel, promptfoo, Inspect, Evalite, Braintrust, LangSmith, Weave,
OpenAI evals, vendor MCP servers): **the operator is never "in Crucible."**
Crucible is infrastructure that surfaces occasionally, not a destination. It
is three things:

1. **An engine** — one binary, installed once, that any repo points at its own
   committed `evals/` directory. **Eval definitions live in the repo they
   measure** (specs + fixtures + reference solutions), PR-reviewed and
   versioned with the code, exactly like tests. This is near-universal
   industry practice and Anthropic's stated guidance (eval tasks contributed
   as PRs). Crucible's own `evals/` is its dogfood set, not the fleet's
   registry. Nobody credible puts definitions in a platform; the documented
   failure mode is silent dataset drift that no one can PR-review.
2. **A globally registered MCP + the `eval-design` skill** — the primary agent
   surface. An agent in any project designs, validates, and runs that
   project's evals without leaving it: author/validate/run against local
   definitions, query/compare against the fleet ledger. The MCP carries the
   verbs; the skill carries the judgment (the industry's own finding: vendor
   MCPs without a process layer produce agents that query scores and invent
   wrong failure taxonomies).
3. **A central run ledger + the review UI on the box** — runs execute locally
   (offline-first is first-class) and land, via thin ingest, in one queryable
   fleet history keyed by config identity + spec hash + project context +
   **git sha**. Results-only-on-laptops kills fleet comparison;
   results-only-central-without-sha makes "which commit broke quality?"
   unanswerable — the ledger refuses both failure modes. The box UI is where
   the operator shows up: review results, adjudicate disputes, watch
   leaderboards, from a phone.

Central truth is a purpose-built ingest service, not multi-writer SQLite —
the local ledger stays as scratch/offline truth and ships run records upward,
the same split every surveyed system converged on (local-first tools for
speed/privacy, centralized experiments for comparison, git-sha as the
linkage).

## What Crucible Should Do

Crucible should support the full eval lifecycle:

- brainstorm, define, and design task families, datasets, fixtures, inputs,
  outputs, and acceptance criteria;
- choose grader types per eval: deterministic checks, computed metrics, agentic
  or model judges, human judgment, or hybrids;
- design rubrics and calibration sets, and calibrate agentic/model judges
  against human labels before their scores are trusted;
- run evaluations to measure and validate the eval itself across models,
  prompts, products, agents, or configurations;
- record every run in a durable, queryable database attached to the benchmark
  and config that produced it;
- show variance, baselines, confidence, disagreement, and cost;
- surface judgment queues to the operator in a delightful, low-friction UI,
  especially on a phone, for the evals that need human judgment;
- collect human labels, preferences, ratings, comments, and adjudications;
- compare runs without hiding uncertainty;
- import eval and benchmark definitions authored elsewhere — public
  benchmarks, other teams' rubrics, other harnesses' task formats — through
  adapters, and run them locally against Crucible's own grader mix,
  calibration, and run database rather than as one-off scripts outside the
  trust layer;
- export eval and benchmark packages, plus defensible run records, to consumers
  like Threshold, Harness Kit, Cerberus, or product repos;
- generate reports that can be used internally, attached to PRs, or published
  when the eval is credible enough.

The mobile judgment surface matters for the evals that require human judgment.
Instead of scrolling social feeds, the operator should be able to review eval
outputs, adjudicate disagreements, rate examples, and improve calibration from
anywhere.

## How Much Judgment Is A Per-Eval Decision

Evals live on a spectrum. Some can be run and judged almost entirely
deterministically, or with a light agentic layer. A good portion require a
non-zero amount of human judgment even when a deterministic and/or agentic layer
does most of the work. Crucible must let each eval declare its own grader mix and
make the human-judgment component cheap, calibrated, and trustworthy — never
hardcode one judgment philosophy across all evals.

## What Excellent Looks Like

An excellent Crucible run makes the measurement story legible:

- the task being measured is specific;
- the eval design names what it can and cannot prove;
- baselines and known-bad examples are included;
- deterministic graders are used wherever possible;
- agentic and model judges are calibrated before their scores are trusted;
- human judgment is captured with enough context to be useful;
- results include uncertainty, cost, failure modes, and examples;
- a future agent can reproduce or audit the run without reconstructing a chat.

The ideal product feels like a lab notebook, workbench, and review queue in one:
serious enough for real decisions, approachable enough to use repeatedly.

## What This Is Not

- Not an optimizer over agent configurations. Threshold owns that; Crucible
  designs the measurement Threshold optimizes against.
- Not a leaderboard factory that publishes scores before the eval design passes
  the smell test.
- Not a generic survey tool with AI branding.
- Not a place to hide judgment-heavy decisions behind one uncalibrated judge.
- Not a dumping ground for every product metric. Crucible is for evals that help
  decide whether behavior improved.
- Not a reinvention of commodity eval infrastructure. Crucible borrows commodity
  execution and ordinary grading where they already plug in, but it must still
  own the benchmark artifact, selected run execution, run records, the
  calibration and trust layer, the human-judgment surface, and the export
  contract.

## Early Shape

Start by making the eval object clear. A minimal useful eval should name:

- task family;
- input and output contract;
- fixture or dataset source;
- grader mix (which graders are deterministic, agentic, or human);
- human-judgment requirements;
- baseline conditions;
- run configuration;
- scoring and aggregation rules;
- confidence or uncertainty reporting;
- export target;
- decision the eval is meant to inform.

The first implementation does not need to solve every eval category. It should
make one real Misty Step eval family easier to design, run, judge, store, and
iterate, then expand from evidence.

The first family is agentic code-review quality: Cerberus-style review and
critic lanes over real diffs, with deterministic checks where possible (a
finding cites a real changed line; dedup; key-match), agentic/model-judge
rubrics where useful (calibrated against human labels), and a phone-friendly
human queue for adjudicating whether findings are correct, important,
duplicated, actionable, or noise.

That family is the right wedge because the surrounding pieces already exist and
are waiting. Verified on 2026-07-01: Threshold's `daedalus` checkout has six live
`pr-review-*` arenas with 35 `tests/expected.json` scorer-key tasks
(`pr-review-v0`, `pr-review-v1`, `pr-review-v2`, `pr-review-security-v0`,
`pr-review-correctness-v0`, and `pr-review-master-v0`); no live arenas currently
exist under the old `pr-review-{verification,product,simplification}` names.
Cerberus produces structured findings via review artifacts, and Crucible now
owns the engine end to end: `crucible author` assembles benchmarks, `crucible
run` makes real model calls across the `prompt_benchmark` and `agentic_judge`
tiers and records every run in a queryable ledger, the adjudication panel's
live writeback loop collects human labels, judge calibration is measured
against those labels, and `crucible export` emits Harbor-importable benchmark
tasks and run records Threshold can consume. See `AGENTS.md`/`SKILL.md` for
the exact command surface and what is still open.

Next families after that:

- Harness Kit primitive evals: raw agent vs Harness Kit vs alternative
  primitive.
- Product behavior evals for Memory Engine or Allie.
- Eval families whose judgment is mostly deterministic or light-agentic, to
  prove the per-eval grader-mix spectrum.

## Decisions For Now

- Crucible owns eval/benchmark definition, design, implementation, selected run
  execution, calibration, run records, judging, reporting, and export. Threshold
  consumes trusted evals and run records to optimize configs. Eval-authoring
  migrates from Threshold into Crucible over time.
- How much judgment an eval needs is a per-eval decision across deterministic,
  agentic, and human layers; most real evals are hybrid and a good portion need
  some human judgment.
- Do not reinvent eval infrastructure. Leverage what already plugs in — the
  existing Threshold arenas/corpus/Harbor format and Cerberus for the code-review
  wedge; existing frameworks (e.g. Promptfoo, Inspect AI) for commodity execution
  and ordinary grading of future families where they fit. Crucible owns the eval
  artifact, selected run execution, the run database, the calibration/trust
  layer, the human-judgment surface, and the export contract; borrowed engines
  sit behind adapters.
- The first concrete eval family is agentic code review and critic quality.
- The first UI should be responsive web, with the human judgment queue designed
  phone-first rather than desktop-shrunken; the next UI milestone is writeback,
  not another static projection.
- The durable, Crucible-owned core (eval object, calibration, uncertainty,
  run storage, export) biases Rust. A thin TypeScript/React layer is justified
  for the judgment UI. Commodity execution and grading can be borrowed, but the
  Crucible-owned run engine and model boundary stay explicit.
- Exports should be boring structured packages aligned to the consumer's
  contract — the Threshold Harbor task-directory format for the code-review
  family: task definition, fixture references, grader manifest, runner hints,
  rubric, baselines, run records, labels, aggregate scores, uncertainty,
  provenance, and the queryable run ids that produced them.
- Form factor (2026-07-09): definitions in-repo per project; runs local-first
  with thin ingest to one central fleet ledger keyed by config identity +
  spec hash + context + git sha; the globally registered MCP (paired with the
  `eval-design` skill) is the primary agent surface; the box UI is the
  operator's review and adjudication surface. See "Form Factor" above.

## Sources

- Operator clarification on 2026-06-28: Crucible should own defining, designing,
  implementing, running, measuring, and iterating evals.
- Operator clarification on 2026-06-28: evals may mix deterministic automated
  judgment, model judgment, and human judgment.
- Operator clarification on 2026-06-28: human-judgment outputs should be
  surfaced through a delightful, approachable UI that works well from a phone.
- Operator clarification on 2026-06-29 (/groom): Crucible is where evals and
  benchmarks are brainstormed, defined, designed, implemented, and iterated;
  Threshold runs Karpathy-style optimization loops that consume Crucible evals;
  eval-authoring should migrate from Threshold into Crucible over time.
- Operator clarification on 2026-06-29 (/groom): how much human vs. agentic vs.
  deterministic judgment an eval needs is a per-eval decision; many evals require
  a non-zero human-judgment component.
- Operator clarification on 2026-06-29 (/groom): keep Crucible as a separate,
  rechartered repo.
- Live-repo evidence (2026-07-01): `daedalus/arenas/pr-review-*` has six live
  arenas and 35 `tests/expected.json` scorer-key tasks; the old
  `pr-review-{verification,product,simplification}` names are not live arenas;
  Crucible has Rust core/CLI/MCP grade/adjudicate/export/run receipts, but no
  live model-call engine yet.
- Live-repo evidence (2026-07-04): the live model-call engine described as
  outstanding above has since shipped, the same day it was written —
  `crucible run` executes `prompt_benchmark` and `agentic_judge` specs through
  real OpenRouter calls, `crucible author` assembles specs without hand-written
  JSON, and the adjudication panel's writeback loop (mounted directly inside
  `crucible serve`) collects human labels. Do not read the 2026-07-01 bullet
  above as current state; `AGENTS.md`/`SKILL.md` carry the live command
  surface.
- Eval-OS intake (2026-07-02): the "What Crucible Should Do" list named export
  but never import, despite the operator's verbatim want to "IMPORT
  benchmarks/evals others have defined and run them locally"
  (`backlog.d/026-external-benchmark-import-adapters.md`) — added as a small
  surgical gap-fill, not a reframing.
- Operator ratification on 2026-07-09 (form factor): "is Crucible something we
  are ever using on its own... or always in the context of another project?
  ... the most critical part of it would be as an MCP." Grounded by the
  2026-07-09 industry survey (scratchpad `formfactor-research/findings.md`,
  Grok lane): definitions-in-repo near-universal (Eve/Vercel `evals/` dirs,
  promptfoo YAML, Inspect `@task`, Evalite `.eval.ts`, OpenAI OSS registry;
  Anthropic: tasks as PRs); results centralize with scale, git-sha as the
  code linkage (Braintrust first-class); vendor MCP servers (Braintrust,
  LangSmith, Phoenix) are ledger-query surfaces while definitions and gates
  stay repo+CLI; no surveyed vendor ships multi-writer SQLite as fleet truth.
