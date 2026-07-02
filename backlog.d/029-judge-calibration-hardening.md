# Harden agentic-judge calibration against the report's checklist

Priority: P2 · Status: pending · Estimate: M

## Goal

`012` made the agentic judge tier real (κ/agreement unlock, judge-gaming
canary refusal). This ticket closes the specific, named gaps between what
`012` shipped and the calibration discipline `ai-evals-benchmarks-report.md`
§6 lays out — Crucible already does more than the report's checklist in some
places (a hard-refuse canary is stricter than anything in §6); these are the
places it does less.

## Gaps (verified against `012`'s progress notes and `crucible/src/spec_run.rs`)

1. **No model-family separation check.** `012`'s own notes flag this
   explicitly: "model-family separation from the generator is not yet
   enforced." Report §6's bias table names self-preference bias
   ("Judge prefers outputs from same model family") with the mitigation
   "Use diverse judges; calibrate against human labels" — today nothing stops
   a judge and candidate from sharing a model family.
2. **No `unknown`/`insufficient_information` judge output.** The judge
   protocol is a strict `VERDICT: PASS`/`VERDICT: FAIL` binary
   (`JUDGE_VERDICT_PROTOCOL`). Report §6 checklist item 8: "Give the judge an
   explicit `unknown` or `insufficient_information` option" — a judge forced
   to guess when evidence is insufficient produces confidently wrong labels
   that look identical to confidently right ones.
3. **No separate false-positive/false-negative tracking.** `CalibrationRecord`
   reports raw agreement + κ + a confusion matrix, but nothing surfaces FP
   rate and FN rate as distinct, trackable numbers (report §6 item 7 and
   §11's "For model-as-judge results, include... False-positive rate,
   False-negative rate").
4. **Calibration is per-run, not a standing licence.** `012`'s notes: "not
   yet aggregated across runs into a standing judge licence." A judge that
   unlocks on one run's small `n` re-proves nothing about drift after a
   judge-model or prompt change (report §6 item 10: "Re-run calibration when
   judge model, prompt, task, or data distribution changes").
5. **No judge cost/latency/failure-rate tracking distinct from candidate
   cost/latency** (report §6 item 11).
6. **No periodic human audit sampling of already-judged examples** (report §6
   item 12) — today's human loop is adjudication of disagreements, not
   spot-audit of agreed-upon judge calls, which is how silent judge drift
   would surface.

## Oracle

- [ ] Judge construction checks (and records) whether the judge model and the
  candidate-generating model share a family; the calibration record surfaces
  this rather than silently allowing it.
- [ ] `JUDGE_VERDICT_PROTOCOL` (or its successor) gains an `UNKNOWN` verdict
  path with a distinct code path from PASS/FAIL — an unknown verdict is
  diagnostic, never silently coerced to pass or fail.
- [ ] `CalibrationRecord` reports FP rate and FN rate as named fields, not
  only aggregate agreement/κ.
- [ ] Calibration unlock state is queryable across runs for a given judge
  (model + prompt + rubric version), not recomputed from scratch and
  discarded each run — i.e. a standing "is this judge currently licensed"
  answer, invalidated when judge model/prompt/rubric changes.
- [ ] Judge-specific cost/latency/failure-rate are recorded per run
  (distinguishable from candidate-side cost/latency already tracked).

## Notes

Prior art: `ai-evals-benchmarks-report.md` §6 ("LLM-as-judge without fooling
yourself") — judge calibration checklist, common judge biases table, judge
output schema — and §11 ("Statistics and interpretation" — "For model-as-judge
results, include..."). Confirms rather than contradicts Crucible's existing
direction (`012`, `VISION.md`'s "agentic and model judges are calibrated
before their scores are trusted"); these are the specific line items `012`
left as "not yet."
