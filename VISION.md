# Crucible

Crucible is an open-source evaluation engine and workbench for people building
with language models and agents. It turns “we need an eval” into a durable,
auditable measurement system: a task and corpus, an execution contract, the
right mix of deterministic, model, and human judgment, calibration evidence,
uncertainty, provenance, and a decision the result can actually support.

It is a long-lived product substrate, not a private script or a time-bounded
experiment. Misty Step uses it across its own repositories, but the public path
is the real path: clone it, bring your own keys, keep definitions beside the
code they measure, run locally, and self-host any shared service you choose to
add. If Crucible only works through Misty Step conventions, it has failed.

## Trust is the product

**Crucible refuses to report a delta it cannot defend.** A rate without an
interval, a model judge without calibration, a comparison that changes two
axes, or a public packet whose provenance and disclosure were not checked are
not small defects. They are failures of the product’s central promise.

Rigor must not make early iteration impossible. Crucible therefore supports a
graduated trust model:

- exploratory evals may run, preserve evidence, and expose what is missing;
- qualified results may support only the claims their sample, grader mix,
  calibration, confounder controls, and uncertainty justify;
- publication is a separate, explicit transition with stricter provenance,
  reproducibility, privacy, and disclosure gates.

The system must make those distinctions legible. “Trusted” is never a vague
blessing on an entire run. A deterministic constraint check can be reliable
while a broad capability claim from the same corpus remains unsupported.
Unknown, underpowered, truncated, provider-error, and genuine-disagreement
outcomes must not be silently collapsed into fail.

## The complete local loop

The core workflow works without an always-on Crucible service:

1. author or import an eval package in the repository that owns the behavior;
2. validate what it measures, what it can prove, and whether its graders work;
3. run it locally with user-controlled credentials and execution boundaries;
4. inspect task evidence, costs, failures, provenance, and uncertainty;
5. calibrate model judges and collect human labels where the eval requires
   them;
6. compare only attributable configurations and state inconclusive results
   plainly;
7. export or publish a versioned, reviewable artifact when it is safe to do so;
8. revise the eval from real failures without erasing its history.

Local data is private by default. Definitions, raw outputs, labels, traces, and
run records do not leave the machine merely because a service is configured.
Synchronization and publication are distinct, explicit actions. A central
ledger can add shared history, remote review, and fleet comparison, but local
success must not depend on it and central failure must not corrupt local truth.

## One core, three equal surfaces

- **CLI:** the human-readable and scriptable contract. Every core lifecycle
  operation must be possible without a particular AI harness.
- **MCP:** parity with the CLI for agents working inside a repository. The MCP
  carries typed verbs and receipts; an eval-design skill may carry judgment,
  but no hidden skill knowledge may be required for correctness.
- **Web UI:** an evidence and judgment surface. It helps people understand
  evals, inspect runs, compare outcomes, and adjudicate disagreements—especially
  from a phone. It must not become a second implementation of authoring or
  orchestration policy.

These surfaces share the same operations, trust rules, and artifacts. Interface
parity does not mean identical presentation; it means no interface silently
changes what a result means.

## A narrow, durable waist

Crucible owns the generic parts of trustworthy evaluation:

- versioned eval, environment, run, trace, calibration, label, comparison, and
  publication artifacts;
- orchestration, provenance, local persistence, uncertainty, and trust-state
  transitions;
- deterministic, model, and human grader composition;
- evidence-rich review and adjudication;
- compatibility and migration rules for public contracts.

Projects own their eval definitions, corpora, reference answers, and the
decisions those evals inform. Reusable eval packages live in ordinary versioned
repositories; Crucible does not need a benchmark registry.

External runners and graders connect through a small, versioned process and
artifact protocol. They may be written in any language. The envelope stays
strict about identity, capabilities, provenance, evidence, and trust; adapters
own runner-specific payloads. Crucible should integrate Promptfoo, Inspect,
Harbor, or project-specific execution where they are useful rather than absorb
their entire frameworks or grow an in-process plugin universe.

## Crucible and Bench

[Bench](https://github.com/misty-step/bench) is Crucible’s strict public
consumer and reference implementation for project-agnostic benchmarks of
models, harnesses, agents, and capabilities.

- Crucible owns the engine, protocols, trust transitions, private ledger, and
  safe publication machinery.
- Bench owns its benchmark definitions, clean-room corpora, references,
  methodology, editorial analysis, public packets, and presentation.
- Bench receives no privileged engine path. An outside user must be able to
  reproduce the same result through the same public Crucible contract.
- A public packet is deliberately disclosed, sanitized or explicitly
  allowlisted, versioned, provenance-bearing evidence—not a copy of whatever
  happened to be in a private run directory.

Optimization systems such as Threshold may consume trusted Crucible results to
search configurations. They do not own the measurement and do not belong
inside Crucible. Keeping the evaluator independent from the optimizer prevents
the builder from grading its own work at repository scale.

## The first proof horizon

Over the next 6–12 months, success is repeated use rather than feature count:

- **Bench** proves public, clean-room, independently reproducible benchmark
  publication through the unprivileged consumer contract.
- **Roster** proves agent and harness evaluation across meaningful
  configuration axes.
- **Memory Engine** proves product-behavior evaluation with a different corpus,
  evidence shape, and grader mix.
- A cold external user can install Crucible, reproduce one Bench result with
  their own key, inspect the evidence, and understand exactly what the result
  does and does not establish.

Those consumers must use the same versioned contracts. Their friction is
product evidence. A private integration, a one-off adapter, or a polished
leaderboard that bypasses calibration does not satisfy the proof.

The current Constraint Gauntlet is a useful deterministic sensor, not proof of
the whole thesis. The trust advantage is demonstrated only when a real family
combines appropriate deterministic checks, independently grounded model-judge
calibration, human adjudication, uncertainty, and replayable evidence—and that
combination changes a decision more reliably than a commodity prompt test.

## Compatibility and adoption

Crucible is deliberately pre-1.0 while Bench, Roster, and Memory Engine expose
the right boundaries. Every artifact is versioned now, and breaking changes
must be explicit and migratable where practical, but bad contracts should not
be preserved merely because they shipped first. After the three-consumer proof,
Crucible should ratify a narrow v1 surface and keep it boring.

Open source, BYOK, and self-hosted use are the default product. A managed
service is a later experiment only if independent adoption demonstrates a real
need for one. Until then, distribution, installation, upgrades, backup,
migration, and clean-room onboarding matter more than tenancy, billing, growth
machinery, or a hosted control plane.

## What Crucible is not

- Not a generic ML experiment tracker, product analytics system, load tester,
  or compliance-certification platform. Its domain is behavioral evaluation of
  language models, tool-using agents, prompts, harnesses, and AI-powered
  product workflows.
- Not a model gateway, dataset warehouse, benchmark registry, or generic
  observability suite. Those are integrations or evidence sources.
- Not an optimizer over agent configurations. It builds and protects the
  measurement an optimizer consumes.
- Not a leaderboard factory. Public ranking is downstream of construct
  validity, measurement resolution, provider identity, and publication safety.
- Not central-first infrastructure. Optional shared services must add value
  without weakening the complete local loop.
- Not an excuse to hide judgment behind one uncalibrated model call or one
  undifferentiated `trusted` flag.
- Not an internal-only tool with public packaging painted on later. Internal
  users dogfood the same contracts everyone else receives.

## What excellent feels like

A good Crucible session ends with less ambiguity than it began with. The task
is specific; the grader mix matches the construct; reference and near-miss
examples expose broken graders; model judges are licensed by independent
labels; human judgment is focused where it changes confidence; every run can be
replayed or audited; costs and provider behavior are visible; comparisons name
their changed axis and noise floor; publication cannot leak private evidence by
accident; and the next iteration follows from observed failures rather than
vibes.

The product should feel like a lab notebook and review bench: rigorous without
being ceremonial, approachable enough to use repeatedly, and honest enough
that “we do not know yet” is a successful result.

This direction was ratified with the operator on 2026-07-11. Revise it when
cross-repo use produces contrary evidence, not when a backlog item needs a more
convenient premise.

