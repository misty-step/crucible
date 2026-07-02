# Investigate and document the pass^5 0.0434 vs 0.333 discrepancy

Priority: P3 · Status: ready · Estimate: S

## Goal

`backlog.d/015-first-real-cerberus-review-benchmark.md`'s Notes flag an
unresolved number mismatch: Cerberus repo's `026-consistency-floor.md` cites
a prior pass^5 measurement "near 0.0434" as the bar to beat, but Crucible's
first live `cerberus-review-quality-v0` run measured pass^5 = 0.333 on
(what the epic believes is) a related arena/candidate. The epic explicitly
says the two numbers are not directly comparable *as reported* and the gap
is unreconciled. This ticket is research/documentation only — read both
sources, trace where 0.0434 actually came from (task count, candidate,
arena version, pass/fail definition), and write up the finding. Do not
change either number or pick a "correct" one; that determination affects
whether Cerberus's blocking-gate criteria are met and is an operator-facing
call.

## Oracle

- [ ] `~/Development/cerberus/backlog.d/026-consistency-floor.md` (or wherever
  it now lives — check `_done/` too) is read in full; the exact source of the
  0.0434 figure (which run, which task set, which candidate, which arena
  version, which pass/fail definition) is identified or confirmed
  unrecoverable.
- [ ] A short reconciliation note is appended to `backlog.d/015-first-real-
  cerberus-review-benchmark.md`'s Notes section (not a rewrite — append,
  dated) stating either: (a) the two numbers now have an identified,
  documented scope difference (task count, corpus version, candidate), or
  (b) the 0.0434 source could not be traced with confidence and why, with a
  named next step for whoever can access the original data.
- [ ] No code changes. No claim is made about which number should gate
  Cerberus's advisory-vs-blocking status — that stays an explicit open
  question for daylight/operator review.

## Notes

Live-code-verified 2026-07-01: `backlog.d/015...md`'s Notes section (dated
2026-07-01) states this exact discrepancy and explicitly frames it as
"real follow-up work before either number can gate anything; flagging the
gap explicitly rather than either dismissing 0.0434 or uncritically treating
0.333 as the new floor." This ticket operationalizes that flagged follow-up
as a bounded investigation task.

**Why:** the epic itself asks for this reconciliation and it is pure
evidence-gathering + documentation — no design/taste call, no code risk, and
it directly de-risks whatever daylight decision follows about Cerberus's
consistency floor.
