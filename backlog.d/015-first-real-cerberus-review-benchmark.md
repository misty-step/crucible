# Ship the first real Cerberus review-quality benchmark

Priority: P1 · Status: pending · Estimate: XL (epic)

## Goal

Create the first production-grade Crucible benchmark: Cerberus review quality,
measuring pass^k consistency and key-recall against adjudicated truth so
Cerberus can earn its path from advisory toward blocking.

## Oracle

- [ ] The benchmark runs Cerberus-style review outputs against an adjudicated
  code-review truth set and reports key-recall with defensible intervals.
- [ ] pass^k consistency is measured across repeated runs/configs, with a
  confidence interval and a noise-floor verdict.
- [ ] The benchmark uses human-adjudicated labels for at least one slice and
  records which judge tiers were trusted vs diagnostic.
- [ ] Results export in a form Cerberus and Threshold can consume for improvement
  loops without turning Cerberus into a repo-level merge gate.

## Verification System

- Claim: Crucible can measure Cerberus review quality on real review tasks.
- Falsifier: the benchmark only replays old Threshold scores, lacks adjudicated
  truth, or cannot distinguish repeated-run consistency from noise.
- Driver: Crucible-authored benchmark over Cerberus artifacts and Harbor scorer
  keys.
- Grader: one-scorer deterministic key recall, human labels, and calibrated
  agentic judge only after `012` unlocks it.
- Evidence packet: benchmark spec, run records, labels, pass^k report, dashboard
  view, and export bundle.
- Cadence: per benchmark version.

## Children

1. Define the benchmark spec and task set from current Cerberus/Threshold data.
2. Collect/adapt adjudicated truth through the human queue.
3. Run repeated Cerberus configs and compute pass^k consistency.
4. Report key-recall, intervals, noise-floor verdict, cost, and examples.
5. Export benchmark/run artifacts to Cerberus and Threshold consumers.

## Notes

Operator decision 2026-07-01: "First real benchmark: cerberus review quality
(pass^k consistency + key-recall vs adjudicated truth) — the eval that gates
cerberus's path to blocking." Until this benchmark is defensible, Cerberus stays
advisory everywhere.
