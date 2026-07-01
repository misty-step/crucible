# Make crucible grade a faithful predictor of Threshold's reward

Priority: P1 · Status: abandoned · Estimate: M

## Goal

Make `crucible grade`'s match logic predict `daedalus-score`'s actual reward
(span containment, false-positive penalty, severity rank) instead of a
category-strict single-line ±tolerance floor, and carry a real span anchor
through adjudication so an accepted defect is not a single-line approximation.

## Oracle

- [ ] `crucible grade`'s recall/precision over a real arena tracks `daedalus-score`'s
  reward within a documented tolerance on ≥5 real tasks (paired, not by luck).
- [ ] An accepted finding exports a defect whose span reflects the finding's real
  region (`line_start..line_end`), so a re-score is robust to the reviewer
  reporting an adjacent line within the region — not just the exact line.
- [ ] grade reports a false-positive count/penalty consistent with `score.rs`, so a
  reported rate is comparable to the optimization loop's objective.

## Notes

Disposition: abandoned by the 2026-07-01 factory groom. The problem is real, but
the "parity by tolerance" shape was the wrong repair. Active replacement:
`013-one-scorer-one-crate.md`, which makes parity hold by construction by moving
the private matcher semantics into one Crucible-owned scorer that Threshold links.

Surfaced by the 2026-06-30 thermonuclear review. Today `crucible export --expected`
correctly extends `tests/expected.json` and the round-trip CLOSES (verified: an
accepted finding flips FP→TP, reward 0.8→1.0 via `daedalus-score`), but two
fidelity gaps remain by design and are documented in `export.rs`:

1. the export span is a single-line under-approximation (`line_start==line_end==line`);
   widening needs a region anchor carried from the Cerberus finding.
2. `crucible grade`'s matcher is a category-strict pre-adjudication FLOOR (file+line
   ±tol, no FP penalty, no severity rank), NOT Threshold's reward — so the grade rate
   is not directly comparable to the optimization objective.

Authoritative scoring stays with `daedalus-score`; this ticket makes Crucible's own
read-side a faithful predictor so it can guide adjudication and pre-flight a config
without round-tripping to Threshold every time. Needs a span/region anchor threaded
from the Cerberus finding through the adapter and the adjudication queue.

Naming: **Threshold** (formerly Daedalus) is the sibling optimization project; its
`daedalus-score` binary keeps the `daedalus` name on disk until the sibling repo
physically renames, so every `daedalus-score` reference above is real and unchanged.
