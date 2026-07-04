# Reconcile the adjudication panel's visual language with Aesthetic

Priority: P2 · Status: open · Estimate: M

## Goal

crucible-031 mounted the adjudication panel's live writeback loop inside
`crucible serve` (`GET /adjudication/panel/<run_id>`, `POST
.../label`), so the panel now renders inside the same application shell as
the rest of `crucible serve`'s cool-monochrome Aesthetic UI. It still ships
its own self-contained warm-parchment/cream stylesheet
(`adjudication_panel.rs`'s inline `<style>` block: `--bg:#f6f1e8`,
`--panel:#fffaf0`, colored pill tags, etc.) rather than the `--ae-*` custom
properties defined in `crucible/src/ui/aesthetic.css`. Mounting the route did
not make the split-personality problem the design audit flagged worse — the
panel's markup and behavior are unchanged from before the mount — but it also
did not fix it: a run opened from inside `crucible serve` still jumps into a
visually distinct product the instant the adjudication panel renders.

## Why this was deferred, not attempted, in crucible-031

`adjudication_panel::render`/`render_live`/`render_live_at` are the one
render path shared across three real consumers with different asset
capabilities:

1. `crucible adjudication-panel --out <dir>` (static files on disk, no
   server at all — opened via `file://` or copied elsewhere);
2. `crucible adjudication-panel --serve` (`adjudication_server.rs`'s own
   tiny HTTP loop, which only exposes `/`, `/queue.json`, `/label` — no
   `/assets/*` route);
3. `crucible serve`'s new mounted route (this is the only one of the three
   that can serve `/assets/aesthetic.css`).

Swapping the panel's inline CSS variables for `<link
rel="stylesheet" href="/assets/aesthetic.css">` would 404 in consumers 1 and
2 (broken/unstyled page, not just a different look), and inlining the whole
of `aesthetic.css` into every rendered panel would bloat every static/served
page for the other two consumers just to satisfy the third. Neither is a
small change, and both risk regressing a shipped, working surface for the
sake of the newest consumer. That tradeoff deserves its own scoped pass, not
a rider on the writeback-mount card.

## Oracle

- [ ] Decide (and record here) which consumer's constraint wins: e.g. teach
      `render_shell` to accept optional inline CSS content so `crucible
      serve` can pass a copy of `aesthetic.css`'s custom properties while
      the other two consumers keep the current self-contained stylesheet, or
      accept a `crucible serve`-only visual identity for the mounted panel
      that borrows Aesthetic's ink/surface/line tokens without the full
      stylesheet.
- [ ] The mounted panel (`GET /adjudication/panel/<run_id>` inside `crucible
      serve`) reads visually as part of the same product as the rest of the
      serve shell: same ink/surface/accent tokens, same button and card
      treatment where it doesn't conflict with the panel's own
      Keep/Nit/Wrong/Noise verdict semantics (color-coded pills stay
      meaningful; they don't need to match `.cru-button` 1:1).
- [ ] `crucible adjudication-panel --out`/`--serve` (the two non-`crucible
      serve` consumers) keep rendering a fully self-contained, correctly
      styled page with no new external asset dependency.
- [ ] Existing `adjudication_panel`/`adjudication_server` unit tests and the
      `crucible-031` mounted-route integration test in `tests/cli.rs` still
      pass unchanged.

## Notes

Flagged during crucible-031 (2026-07-04): the card's own scope note says "if
it's not tractable in scope, note it as a follow-up rather than attempting a
full redesign" — this is that follow-up.
