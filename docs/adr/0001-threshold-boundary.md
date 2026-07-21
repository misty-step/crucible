# ADR 0001 — The Crucible/Threshold boundary: measurement stays sovereign

Status: PROPOSED (drafted by the lead 2026-07-04; awaiting operator
ratification — powder card `crucible-036`)
Deciders: operator (ratifies), fable-lead (drafts)
Evidence: the crucible-036 investigation packet (2026-07-04), reproduced in
summary below with file citations; `backlog.d/007`, `backlog.d/028`,
`docs/daedalus-eval-authoring-map.md`, threshold `VISION.md` +
`docs/048-cerberus-rd-lab-context.md` + `docs/crucible-eval-optimization-contract.md`.

## Context

Two open tickets point the repo-boundary question in opposite directions.
`007` (P2, XL, mid-flight: child 1 delivered, child 2 blocked) migrates
eval-authoring **out of Threshold into Crucible**. `028` (P2, filed a week
later, zero investigation executed against its own oracle) asks whether
Threshold's config-search Lab stage should instead fold **into Crucible**.
Every week both stay open, both repos add surface on an identity that could
flip — 029/030/031 here, 061–068 there.

Facts that bound the decision:

- **The boundary has been ratified twice.** 2026-06-29 recharter ("Crucible
  owns the eval/benchmark as a durable artifact… Threshold runs optimization
  loops that consume Crucible's trusted evals; eval-authoring machinery
  migrates from Daedalus into Crucible over time") and the 2026-07-01 factory
  decision ("Threshold parked behind Crucible") with explicit reentry
  criteria: answer-key grading through a shared scorer, self-verdicts never
  the objective, duplicate optimizer stacks collapsed.
- **Why Threshold is parked is the load-bearing fact.** The optimizer's
  plumbing proof (threshold PR #26 / ticket 061) scored candidates partly on
  their own self-report — exactly the failure the parked status exists to
  stop. The measurement organ's entire product thesis ("refuses to report a
  delta it cannot defend") depends on the optimizer not being able to reach
  into it.
- **The live coupling runs backward from the target story.** Crucible's
  committed specs read Threshold's `arenas/` off sibling disk paths
  (`evals/cerberus-review-quality-v0.json` → `../../daedalus/...`), and
  Crucible had to ship an alias-resolution shim (`spec_run.rs`
  `daedalus_to_threshold_raw`) just to survive Threshold's 07-01 on-disk
  rename. Today Crucible is the dependent party.
- **The extraction is already half-happening, in the worst form.** Crucible's
  `grade.rs`/`export.rs`/`key.rs`/`measure` independently re-implement
  Threshold's matcher, adjudication round-trip, answer-key parsing, and
  eval-side stats. Two copies of the matcher logic exist right now. Unplanned
  duplication is the most expensive way to migrate.
- **Threshold's own docs disagree with each other** (same-day: `VISION.md`
  says Crucible owns arenas; `docs/048` says Threshold owns arenas;
  `docs/crucible-eval-optimization-contract.md` says transitional). The
  "ownership handshake" 007 waits on is real work, not ceremony.
- **Nothing anywhere argues for fold-in.** No Threshold ticket in 034–068
  contests eval ownership or proposes absorption; all treat Crucible-owned
  evals as the target and harden the consumption side. 028's fold-in option
  has no evidence packet, no operator signal, and its own text flags the
  scope explosion (Crucible is 11.6k LOC and coherent; the Lab stage brings
  GEPA mutation, Pareto archives, ASHA/Hyperband, seed/swarm/lineage).

## Decision

**The boundary holds and hardens. Crucible owns measurement end to end —
including eval authoring. Threshold stays a separate repo that consumes
Crucible's trusted evals and owns only optimization. `028` resolves to "stay
separate" now, by ruling. `007` continues as the single active migration,
re-sequenced below.**

The deciding argument is not code volume; it is adversarial integrity. A
measurement organ that shares a codebase, gates, and release cadence with the
optimizer it certifies is the builder-grades-own-work failure at repo scale —
and this fleet has already been bitten by exactly that shape (self-verdict
scoring is why Threshold is parked). Independence is cheap to keep and
expensive to rebuild; consolidation buys a shared crate's worth of
convenience and sells the refusal posture that is Crucible's entire product.

### The boundary, stated once

| Owns | Crucible | Threshold |
|---|---|---|
| Eval/benchmark definition (specs, arenas, task dirs, answer keys, taxonomies) | ✔ | |
| Corpus governance (holdout ledgers, contamination, burn discipline) | ✔ | |
| Running, grading, calibration, adjudication, export | ✔ | |
| Measurement stats (rates, CIs, agreement, noise floors) for eval verdicts | ✔ | |
| Config-space search (GEPA, Pareto, ASHA, seed/swarm/lineage) | | ✔ |
| Loop-side stats (cluster-robust reward deltas) for search decisions | | ✔ |
| Harbor runner / sprite execution of candidates | | ✔ |
| SHARED, contract-pinned: Harbor task-directory format; scorer binary (design: Crucible; build/run: Threshold); Cerberus handoff packet schemas; holdout write-back protocol | ✔ contract | ✔ consumer |

Consumption direction is one-way: **Threshold reads Crucible exports.
Crucible never reads Threshold's tree.** The current inversion (committed
specs on `../../daedalus/...` sibling paths) is a named defect this ADR
retires family-by-family, not a pattern.

### What this resolves

- **028 → closed, "stay separate."** The "partial" option (shared
  stats/measure crate) is also rejected for now: the two stat surfaces serve
  different masters (eval verdicts vs. search decisions), and a shared crate
  across a sovereignty boundary is a speculative abstraction with exactly two
  semi-overlapping consumers. Definitions may converge by documented contract;
  implementations may deliberately duplicate, each proven by its own tests.
  **Reopen trigger (record, don't relitigate):** a third consumer appears, or
  a real divergence bug ships because the two implementations disagreed on the
  same statistic.
- **007 → the one migration, re-sequenced:**
  1. **Handshake = doc convergence (child 2, unblocked by this ADR).** This
     ADR merges in Crucible; a matching Threshold PR fixes `docs/048`'s
     contradiction and points at this file as the single boundary source.
     Both repos' AGENTS/VISION cite it rather than restating it.
  2. **Invert the dependency for one family (child 3, the proof).** The
     pr-review arenas + taskspec + answer keys for the code-review family move
     into Crucible; Threshold consumes them through the Harbor export.
     Success = Crucible's flagship spec runs on a cold clone with no sibling
     checkout, and the alias shim is unnecessary for that family.
  3. Corpus governance moves (child 4), Threshold narrows (child 5), and the
     duplicated matcher logic collapses to the Crucible copy.
- **032 → verify-and-close.** Its oracle appears already met in code
  (`resolve_spec_path_with_alias` + distinct-warning tests landed in PR #84);
  the ticket looks stale. Confirm and archive; the shim itself sunsets as
  step 2 completes per family.

### Riders (small, overdue, independent of the ruling)

- Re-sync Crucible's own `AGENTS.md`, `VISION.md`,
  `docs/daedalus-eval-authoring-map.md`, and both committed eval specs off the
  dead `daedalus` name (Threshold physically renamed 2026-07-01; Crucible's
  docs still assert the rename hasn't happened).
- Threshold-side: reconcile `docs/048-cerberus-rd-lab-context.md` with
  `VISION.md` (the step-1 handshake PR).

## Consequences

- Two identity-shaped tickets collapse to one migration with a proof-shaped
  milestone; neither repo keeps building on an ambiguous boundary.
- Crucible accepts custody of ~67M of arena fixtures and the authoring CLI
  surface (~5.5k LOC worth of responsibility) as deliberate scope — this is
  the recharter's stated intent, not drift.
- Threshold's reentry criteria are unchanged and now have a concrete
  measurement seam to re-enter through (step 2's Harbor export).
- Reversibility: this ADR changes documents and sequencing today; the
  physical moves happen family-by-family under 007, each step shippable and
  individually reversible until the Threshold-side deletion (child 5), which
  is gated on the family running end-to-end from Crucible.

## Ratification

Operator ratifies by comment on powder card `crucible-036` (or bridge
answer). On ratification: merge this ADR, mark `028` abandoned-with-pointer,
update `007` child 2 to in-progress, file the Threshold handshake PR.
