# Delightful phone-first adjudication queue

Priority: P1 · Status: in-progress · Estimate: L (epic)

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
- [x] The current static `adjudication-panel` becomes an actual writeback loop;
  CSS-only buttons are not acceptable completion evidence. (`--serve`, see
  progress note — minimal, not yet optimistic/offline-tolerant.)

## Children (ordered)

1. ✅ Schema-driven card + diff render (static fixture) — shipped as the
   existing static panel.
2. Four-verdict tap bar + auto-advance + Undo + optimistic save (partial — see
   progress note: real tap-to-save landed, not yet optimistic/auto-advance/Undo).
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

**Progress 2026-07-02 (overnight):** the minimal writeback loop is real. `crucible
adjudication-panel --serve [--port N] [--labels PATH]` starts a small
`std::net`-only local HTTP server (no framework, no new dependency;
`crucible/src/adjudication_server.rs`) alongside the existing static render:
`GET /` and `GET /queue.json` serve the queue (including labels applied so far
this session, so a resumed/restarted session shows prior work); `POST /label`
takes `{finding_id, verdict, in_scope, latency_ms}`, mints a `Label` through the
*same* `apply_label` path `crucible adjudicate --apply` already used, and
persists the accumulated labels as a `crucible.label.v1` JSON array — the exact
shape `--apply` reads, so a served session's output re-enters the headless loop
with zero conversion (verified live: `curl -X POST .../label` → the label lands
in `--labels`'s file → `crucible adjudicate --apply <that file>` reads it back
and shows it in the queue). `saw_grader_before_commit` is always `true` — the
panel shows the deterministic grader's context (category, recoverable-against
rows) inline before every verdict, so that is the honest recorded value, not a
default to override later. `adjudication_panel::render_live` adds
`data-finding-id`/`data-verdict` attributes and an inline `fetch()`-based
script; the original static `render`/`write_panel` (no `--serve`) are
byte-for-byte unchanged and their test is unchanged.

Tested at two levels: unit tests for the label-minting/persistence logic
(mint+persist, last-write-wins on a repeat verdict, unknown-finding-id
rejection, missing-labels-file resume), and one test that binds a real
ephemeral TCP listener, drives the actual accept loop in a background thread,
and issues real HTTP requests over `TcpStream` — proving the wire format
round-trips, not just the Rust-level handler logic.

**Explicitly not done** (this was the minimal-loop slice, not the full epic):
optimistic UI (the button disables and waits for the server round-trip, no
speculative advance), Undo, auto-advance to the next item, secondary chips,
bounded-session progress/finish state beyond the existing item counters, blind
gold + calibration report, offline tolerance, and any mobile-specific polish.
Concurrency: the server is deliberately single-connection-at-a-time — correct
for one judge, not for two people adjudicating the same queue at once.
