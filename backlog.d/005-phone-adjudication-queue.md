# Delightful phone-first adjudication queue

Priority: P1 · Status: ready · Estimate: L (epic)

## Goal

A thin phone/web consumer of the judgment-queue artifact that lets the operator
adjudicate a code-review finding in under five seconds and writes labels back to
Crucible — the human-judgment tier for evals that need it, deliberately the
opposite of an infinite feed.

## Oracle

- [ ] Operator clears a 30-item session on a phone: one snap verdict per item
  (Keep / Nit / Wrong / Noise) + prefilled sub-chips, sub-second optimistic
  advance with Undo, bounded with a satisfying finish state, resumable,
  offline-tolerant.
- [ ] Blind-first: grader verdict revealed only after commit (always blind for
  gold); the session ends with an agreement-with-gold calibration report + a
  disagreement mini-queue.
- [ ] The UI adds zero new core design — it renders from the embedded
  `schema_version` and writes `Label`s back through the contract from 002/004.
- [ ] The current static `adjudication-panel` becomes an actual writeback loop;
  CSS-only buttons are not acceptable completion evidence.

## Children (ordered)

1. Schema-driven card + diff render (static fixture).
2. Four-verdict tap bar + auto-advance + Undo + optimistic save.
3. Secondary chips — duplicate-confirm, severity, voice comment, defer.
4. Bounded session — progress + finish state + resume.
5. Blind gold + calibration report + disagreement mini-queue.
6. Offline / resume + writeback sync.
7. Calm anti-doomscroll polish. Non-goals (explicit): no infinite feed, no streak
   guilt, no variable-reward bait.

## Notes

Gate the build behind "one adjudication loop works from the CLI" (wedge 002).
Human and model judge share ONE `{verdict, severity}` schema so the queue doubles
as calibration data. Capture `latency_ms` + `saw_grader_before_commit` to record
the conditions of judgment for calibration validity. The five vision dimensions
(correct/important/duplicate/actionable/noise) collapse into the four-verdict
primary + chips so labeling is one thumb gesture, not a form.

**Update 2026-06-30:** UNBLOCKED — the gating prereq ("one adjudication loop works
from the CLI", wedge 002) is met: `crucible adjudicate`/`export` close the headless
loop and the Threshold round-trip is lead-verified. The schema the UI renders
(`crucible.judgment_queue.v1` reading into `crucible.label.v1`) is shipped and
stable. This is now the headline next pickup — the first time human judgment flows
through Crucible, and what produces the labels the κ judge-calibration gate (003/002.6)
is blocked on.

**Factory groom 2026-07-01:** this is the human tier inside
`012-three-judge-tiers-real.md`. Ship the minimal writeback loop first; React or
polish is secondary to collecting valid labels.
