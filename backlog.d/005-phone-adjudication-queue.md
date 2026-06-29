# Delightful phone-first adjudication queue

Priority: P2 · Status: pending · Estimate: L (epic)

## Goal

A thin React consumer of the judgment-queue artifact that lets the operator
adjudicate a code-review finding in under five seconds on a phone — the
human-judgment surface for evals that need it, deliberately the opposite of an
infinite feed.

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
