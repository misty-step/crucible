# Code-review eval: the first benchmark Crucible owns end to end

Priority: P1 · Status: ready · Estimate: XL (epic)

## Goal

Prove Crucible's model on one real family: define, calibrate, and iterate the
agentic code-review eval; industrialize its adjudication; bootstrap labels for
real diffs; and emit a Harbor benchmark Daedalus can re-score and optimize
against.

## Oracle

- [ ] One real disputed Cerberus finding set is adjudicated through Crucible's
  CLI (the five labels) and exported as a Harbor `adjudications.md`
  key-extension / arena version bump that Daedalus re-scores against.
- [ ] At least one currently-blocked Daedalus arena
  (`pr-review-{simplification,product,verification}`) gains ≥5 labeled,
  calibrated Harbor tasks.
- [ ] Any model/agentic judge ships with a measured judge-vs-human agreement
  (Cohen's κ) and is gated, not assumed.
- [ ] Per-config code-review rates are reported with a Wilson interval and a
  paired (McNemar) comparison; a delta inside the noise floor prints "inside
  noise floor", not a winner.

## Verification System

- Claim: Crucible turns raw Cerberus findings over real/disputed diffs into
  adjudicated, calibrated, Daedalus-importable benchmark tasks.
- Falsifier: the round-trip fails — Daedalus cannot re-score the emitted Harbor
  artifact, or the judge ships uncalibrated, or a "winner" is declared inside the
  CI.
- Driver: `crucible` CLI over a real Daedalus disputed-finding record +
  `cerberus review-diff --base --head --json`.
- Grader: deterministic (anchor cites a real changed line; dedup; key-match) +
  calibrated model-judge (κ-gated) + human adjudication (five labels).
- Evidence packet: emitted Harbor task dir + `adjudications.md` diff +
  calibration record (κ, confusion matrix) + scored report with CIs.
- Cadence: per child; re-run on each arena version bump.

## Children (ordered)

1. **(SPIKE — gating)** Confirm the true critical path:
   `cerberus review-diff --base --head --json` emits structured findings, and a
   real Daedalus disputed-finding / holdout record exists to adjudicate
   (`arenas/pr-review-v0/holdout-ledger.md`). Output: go/no-go + concrete inputs.
2. **Corpus + Harbor adapter** — ingest Cerberus `Finding`/`ReviewArtifact` and
   Daedalus disputed-finding records; pin to the Harbor task-directory contract;
   confirm round-trip import into Daedalus.
3. **Deterministic pre-graders (Rust)** — schema-valid; anchor cites a real
   changed line; dedup; key-match by file/line/category. Trim the queue to
   disputed / low-confidence findings only.
4. **Finding-judgment record + CLI adjudication queue** — five labels
   (correct/important/duplicate/actionable/noise) + rationale; emits the
   judgment-queue artifact contract so the phone UI (005) is later a thin
   consumer.
5. **Export** — `adjudications.md` key-extension (ACCEPT) or new Harbor task;
   calibration record (judge vs human κ); scored report (Wilson CI + McNemar +
   noise-floor verdict).
6. **Borrowed/agentic model-judge behind the calibration gate.** Phone UI is a
   later epic (005).

## Notes

Built on verified live evidence (see `VISION.md` Sources). Scope guard: Crucible
designs and calibrates the measurement; it does NOT run the optimization search
loop — that is Daedalus. Draws uncertainty/calibration primitives from epic 003
and types from epic 004; do not duplicate them here. Make-or-break is the
boundary and the data, not the machinery: keep the wedge scoped to
trust/adjudication/calibration/real-diff bootstrap.
