# Collapse code-review scoring into one Crucible-owned scorer

Priority: P1 · Status: in-progress · Estimate: M

## Goal

Remove duplicate code-review matching semantics by making `crucible-core` the
single scorer Threshold links, instead of tolerance-matching private
implementations across repos.

## Oracle

- [ ] `crucible/src/spec_run.rs` no longer owns a private matcher for expected
  defects; it calls the scorer in `crucible-core`.
- [ ] The scorer supports the semantics Threshold needs: span containment,
  category matching, severity rank, false-positive penalty, and exported spans
  that do not collapse accepted findings to a single line.
- [ ] Threshold/Daedalus can link the same scorer crate or a shared extracted
  crate, and a cross-repo golden fixture proves both binaries produce the same
  counts/reward.
- [ ] `backlog.d/_done/008-grade-daedalus-reward-parity.md` stays archived as the
  superseded parity-by-tolerance shape.

## Verification System

- Claim: Crucible and Threshold share scoring semantics by construction.
- Falsifier: the same fixture can produce different match/reward counts in
  `crucible grade` and `daedalus-score`.
- Driver: shared golden code-review fixtures scored by both binaries.
- Grader: equality assertions over matched ids, false positives, severity
  handling, recall, and reward.
- Evidence packet: fixture outputs from both binaries plus the cross-repo test
  transcript.
- Cadence: every scorer semantic change.

## Children

1. ✅ Move span/category/severity/FP reward semantics into `crucible-core`.
2. ✅ Replace `spec_run.rs` private matcher calls with the core scorer.
3. Carry real region/span anchors through adapter → queue → export.
4. ✅ Add Crucible golden tests for scorer semantics.
5. Coordinate a Threshold PR that links the same scorer and adds cross-repo
   fixture equality.

## Notes

This replaces old backlog `008`. The boundary decision stays split: Crucible owns
measurement; Threshold optimizes configs against the trusted measurement. The
implementation boundary changes so scoring code, not prose parity, is the
contract.

Progress 2026-07-01: children 1, 2, and 4 landed. `crucible-core/src/key.rs`
now owns `score_against_expected_key`/`SpanGrade` — the span-containment +
exact-category + severity-rank-floor + false-positive-count matcher that
`daedalus-score` implements, extracted verbatim from the private duplicate
that used to live in `crucible/src/spec_run.rs` (`score_against_expected_key`,
`defect_matches`, `severity_matches`, `severity_rank`, and the local
`SpanGrade` struct — all deleted). `spec_run.rs`'s `grade_key_recall_task` now
calls `crucible_core::score_against_expected_key` directly; behavior is
unchanged (moved, not rewritten) and covered by 10 new golden tests in
`crucible-core` (span-inside vs. just-outside with no ±tolerance, exact
category requirement, severity-floor accept/reject including an unranked
label, absent-floor accepts any severity, greedy first-unclaimed-defect
matching, and an extra-candidate false positive). This scorer is distinct from
`crate::key_match`/`grade` (the point-and-tolerance approximation `crucible
grade` uses over Cerberus artifacts) — `Defect::to_key_finding`'s doc comment
already named that approximation and pointed at "the authoritative span match
stays with `daedalus-score`"; that authoritative match now also lives in this
crate. Remaining: child 3 (real region/span anchors through the Cerberus
adapter → judgment queue → export path, a separate pipeline from this
key-recall runner) and child 5 (the cross-repo Threshold PR — out of this
repo's scope, needs coordination with the threshold/daedalus lane).
