# Measurement rigor core: refuse to report a delta you cannot defend

Priority: P1 · Status: in-progress · Estimate: L (epic)

## Goal

Build the trust machinery that makes Crucible more than a runner — calibration,
agreement, uncertainty, paired comparison, and provenance — reusable across every
eval family.

## Oracle

- [ ] Every reported rate carries an interval (Wilson for binary/small-n;
  bootstrap BCa for composite metrics).
- [ ] A model/agentic judge unlocks only above a measured judge-vs-human
  agreement threshold on a κ-validated human set; its confusion matrix is stored
  and used to bias-correct scores.
- [ ] Two configs are compared paired (McNemar); a verdict is refused (prints
  "inside noise floor") when p > α or the effect sits inside the CI.
- [ ] A pre-run power check warns when the fixture set is underpowered for the
  target effect.
- [ ] Every run persists an Evaluation Card (model+version, temperature, seed
  count, prompt/rubric hash, fixture refs, raw per-item judgments, cost,
  timestamp) that reproduces the verdict with zero chat context.

## Children (ordered)

1. ✅ Provenance / Evaluation-Card types — delivered 2026-06-30; still need
   persistence through real runs (010/011).
2. ✅ Uncertainty primitives — Wilson + seeded bootstrap landed; keep wiring them
   into every reported aggregate.
3. Baseline + known-good/known-bad anchors + judge sanity check (judge must fail
   known-bad and pass known-good or the measurement surface is broken).
4. Inter-annotator κ on a double-labeled subset — gates the *rubric*, not just
   the judge.
5. Judge calibration gate — agreement + confusion matrix + bias panel (position,
   verbosity, self-preference; forbid judge == generator family).
6. ✅ Paired comparison + noise-floor decision gate + pre-run power sizing
   primitives — delivered 2026-06-30; still need pre-run invocation in the runner.

## Notes

Methodology evidence (cited in groom report): calibrated judges reach >80%
human-agreement; position bias up to ~75% first-slot; Wilson preferred at small
n / extreme p; McNemar for paired binary data. Single-operator κ caveat: needs
≥2 labelers or an objective anchor (e.g. seeded-defect recall) to bootstrap —
coordinate with the wedge (002). This epic is the durable moat; the wedge
consumes its first primitives. Promote children to `Status: ready` with their own
verification systems as they are picked up.

**Update 2026-06-29:** the first primitives landed in `crucible-core::measure`
(Wilson interval, proportion, percent-agreement) via the wedge build. Hardening
follow-ups from thermonuclear review: guard `wilson_interval` for successes>n;
make `agreement` reject length-mismatch (return `Option`) so a misaligned
judge/human pair cannot silently cross an unlock threshold; serialize the
`--json` rate as `null` (not `0.0`) at n==0 ("no data" ≠ "0%"); surface a
dropped-invalid-finding count. Fold these into 003's children.

**Update 2026-06-30:** hardening + new primitives shipped — `wilson` successes>n
guard; `agreement`/`cohen_kappa` return `Option` (None on misaligned/empty, so a
judge cannot silently cross a threshold); McNemar paired comparison + noise-floor
refusal (the "refuse a delta you cannot defend" gate — p∈[0,1] after fixing an
erfc tail-overshoot that produced p>1); power/sample-size check; a seeded,
reproducible bootstrap; and the `Provenance`/`EvaluationCard` + `CalibrationRecord`
types (with finiteness + schema-version guards). Remaining: wire the Evaluation Card
into persisted runs; the κ judge-calibration *unlock* needs real human labels
(002.6 / 005). Children 2 + 6 of this epic now have shipped primitives.

**Factory groom 2026-07-01:** keep this epic as the measurement-kernel spine, not
the top pickup. The active engine work (`010`) must persist Evaluation Cards,
honor or refuse declared confidence, and call the power-warning primitive before
claiming a comparison.
