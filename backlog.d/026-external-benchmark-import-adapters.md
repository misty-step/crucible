# Import external benchmarks/evals through adapters, not one-off scripts

Priority: P1 · Status: pending · Estimate: L (epic)

## Goal

Let Crucible pull in benchmarks/evals other people already defined — public
suites, other teams' rubrics, other harnesses' task formats — and run them
locally against Crucible's own grader mix, calibration, run database, and
dashboard, instead of every external benchmark becoming a bespoke script that
never touches the trust layer. This is one of the operator's three explicit
"functional application" requirements (define/design/implement/run/evaluate
*and* import others' evals) and today Crucible only exports (VISION.md
`emit Harbor-importable benchmark tasks`); there is no import path at all.

## Oracle

- [ ] A `crucible import <adapter> <source>` (or equivalent) command projects
  an external benchmark's tasks into Crucible's own `EvalSpec`/corpus shape
  (`crucible-core/src/spec.rs`), tagged with `source: external` provenance
  (adapter name, source version/commit, import date) so imported evals are
  never silently indistinguishable from Crucible-authored ones.
- [ ] At least one adapter runs end-to-end: import → `crucible run` → a row in
  the run database (`011`) → visible in `crucible runs list`/dashboard (`009`)
  next to native benchmarks.
- [ ] Import is total and honest: tasks the adapter cannot map cleanly are
  surfaced (skipped-and-counted, per the `009` ingest precedent — "no silent
  loss"), not dropped.
- [ ] Follows the VISION.md decision to not reinvent eval infrastructure —
  adapters are thin projections into Crucible's own spec/grade/run/store
  pipeline, not new parallel execution engines.

## Children (proposed first adapters — concrete, not exhaustive)

1. **SWE-bench (or SWE-bench Verified/Lite) task format** — closest fit to
   Crucible's own flagship family (agentic code review/patch quality over real
   repos); a real second data point for the agentic-judge and deterministic
   key-match graders already built for Cerberus.
2. **lm-evaluation-harness YAML task format** — broadest catchment: importing
   this one schema opens the door to most of the academic/public-benchmark
   corpus (MMLU-style multi-choice, short QA) with one adapter instead of one
   per benchmark.
3. **Inspect AI eval/task format** — the report's recommended harness for
   reproducible custom and agent evals; its task+sample+scorer shape is close
   to Crucible's own `EvalSpec`, and its trace/log format is a plausible
   second input to the trace-inspection work in `030` (agent/tool-call
   trajectories, not just prompt/response).

## Notes

Prior art: `ai-evals-benchmarks-report.md` §9 ("Tool matrix") and §10
("Public benchmark map" — benchmark categories table + "How to borrow from
public benchmarks") name the concrete external formats and warn that public
benchmarks are vulnerable to distribution mismatch, leaderboard optimization,
contamination, and irrelevance to the product's actual risk profile — import
should carry that caveat forward as `source: external` provenance, not present
imported scores with the same trust posture as a Crucible-native, calibrated
benchmark.

This is additive to VISION.md's export-only "eval and benchmark packages...
to consumers like Threshold" line — see the small VISION.md edit in this same
intake pass adding the import direction to "What Crucible Should Do."
