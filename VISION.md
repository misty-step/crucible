# Crucible Vision

Crucible exists to make evals easier to design, run, judge, understand, and
improve.

It is the eval workbench for Misty Step's agent and product experiments. A good
eval is not just a prompt and a score. It is a task definition, fixture set,
execution plan, grader mix, human calibration path, result surface, and iteration
loop that tells us whether a model, agent, prompt, tool surface, product change,
or workflow is actually getting better.

The product should help turn "we need an eval for this" into a concrete,
auditable measurement system: what task are we measuring, what counts as good,
what can be checked deterministically, what needs model judgment, what needs
human judgment, how confident are we, and what should change next?

## Why This Exists

Misty Step needs real science around AI systems. Agent work is too nonlinear for
vibes: a new model, a different system prompt, a tool allowlist, a subagent, a
reasoning budget, or a UI change can feel better while producing worse outcomes.
Without evals, every improvement claim collapses into taste, demos, and recent
memory.

Crucible is where those claims get tested. It should make eval design accessible
enough to use often, rigorous enough to trust, and humane enough that the
operator can contribute judgment without turning it into a miserable chore.

## The Role In The Constellation

Crucible owns eval design and eval operations.

- Harness Kit carries reusable agent primitives and portable eval contracts for
  primitive-level claims.
- Daedalus optimizes agent configurations against trusted eval surfaces.
- Product repos keep project-specific evals close to the behavior they care
  about.
- Crucible helps define, run, review, compare, calibrate, export, and iterate
  those evals.

If the question is "which agent configuration wins against this trusted
measurement surface?", that is Daedalus. If the question is "what should the
measurement surface be, and do we trust it?", that is Crucible.

## What Crucible Should Do

Crucible should support the full eval lifecycle:

- define task families, datasets, fixtures, inputs, outputs, and acceptance
  criteria;
- choose grader types: deterministic checks, computed metrics, model judges,
  human judgment, or hybrids;
- design rubrics and calibration sets;
- run evaluations across models, prompts, products, agents, or configurations;
- show variance, baselines, confidence, disagreement, and cost;
- surface judgment queues to the operator in a delightful, low-friction UI,
  especially on a phone;
- collect human labels, preferences, ratings, comments, and adjudications;
- compare runs without hiding uncertainty;
- export eval packages to projects like Harness Kit, Daedalus, or product repos;
- generate reports that can be used internally, attached to PRs, or published
  when the eval is credible enough.

The mobile judgment surface matters. Instead of scrolling social feeds, the
operator should be able to review eval outputs, rate examples, resolve
disagreements, and improve calibration from anywhere.

## What Excellent Looks Like

An excellent Crucible run makes the measurement story legible:

- the task being measured is specific;
- the eval design names what it can and cannot prove;
- baselines and known-bad examples are included;
- deterministic graders are used wherever possible;
- model judges are calibrated before their scores are trusted;
- human judgment is captured with enough context to be useful;
- results include uncertainty, cost, failure modes, and examples;
- a future agent can reproduce or audit the run without reconstructing a chat.

The ideal product feels like a lab notebook, workbench, and review queue in one:
serious enough for real decisions, approachable enough to use repeatedly.

## What This Is Not

- Not an optimizer over agent configurations. Daedalus owns that.
- Not a leaderboard factory that publishes scores before the eval design passes
  the smell test.
- Not a generic survey tool with AI branding.
- Not a place to hide judgment-heavy decisions behind one model judge.
- Not a dumping ground for every product metric. Crucible is for evals that help
  decide whether behavior improved.

## Early Shape

Start by making the eval object clear. A minimal useful eval should name:

- task family;
- input and output contract;
- fixture or dataset source;
- grader mix;
- human-judgment requirements;
- baseline conditions;
- run configuration;
- scoring and aggregation rules;
- confidence or uncertainty reporting;
- export target;
- decision the eval is meant to inform.

The first implementation does not need to solve every eval category. It should
make one or two real Misty Step evals easier to design and judge, then expand
from evidence.

Good first candidates:

- Harness Kit primitive evals: raw agent vs Harness Kit vs alternative
  primitive.
- Daedalus search eval packages: task/eval surfaces that Daedalus can optimize
  against.
- Product behavior evals for Memory Engine, Allie, or agentic review work.

## Open Questions

- What is the first concrete eval family Crucible should support?
- Should the first UI be web-first, mobile-first, or a responsive web app that
  treats mobile review as a first-class surface?
- How much runner logic belongs here versus in project repos?
- What should the export format look like so Harness Kit and Daedalus can
  consume eval packages without tight coupling?
- Which parts must be Rust from the start, and which UI surfaces justify a
  small TypeScript/React layer?

## Sources

- Operator clarification on 2026-06-28: Crucible should own defining, designing,
  implementing, running, measuring, and iterating evals.
- Operator clarification on 2026-06-28: evals may mix deterministic automated
  judgment, model judgment, and human judgment.
- Operator clarification on 2026-06-28: human-judgment outputs should be
  surfaced through a delightful, approachable UI that works well from a phone.
- Operator clarification on 2026-06-28: Daedalus should use evals to optimize
  agent configurations rather than own the whole eval product.
