# Test coverage: adjudication server's GET / live HTML render

Priority: P2 · Status: done · Estimate: S

## Goal

`crucible/src/adjudication_server.rs` (merged tonight, PR #68 — "CSS-only
buttons finally do something") has 5 tests covering `/queue.json` and
`/label`, but none exercise `GET /` (the actual panel HTML a judge opens in a
browser) or confirm it reflects labels already applied this session.

## Oracle

- [x] A new test drives the real `accept_loop` (same pattern as
  `live_server_serves_the_panel_and_accepts_a_real_http_label_post`, line
  432) issuing `GET / HTTP/1.1`, and asserts the response is `200`,
  `Content-Type: text/html...`, and the body contains the queue item's
  `finding_id`.
- [x] A second assertion (same test or a follow-up one) applies a label via
  `POST /label` first, then re-requests `GET /`, and asserts the rendered HTML
  reflects the applied label (e.g. contains the labeled verdict or an
  updated progress count) — proving `render_live` is actually re-invoked with
  current `labels`, not a stale snapshot.
- [x] `cargo test --all` passes.

## Notes

Live-code-verified 2026-07-01: `crucible/src/adjudication_server.rs:141-152`
handles `GET /` and `GET /index.html` identically to `/queue.json` (both
clone `labels` into the render queue), but `rg '"GET", "/"' crucible/src/
adjudication_server.rs` shows no test hits that route — only `/queue.json`
and the 404 path (`/nope`) are exercised over the real socket. Given this
module is the actual human-facing surface backlog `005` was blocked on, the
route a judge's browser hits by default should have direct proof, not just
its JSON sibling.

**Why:** matches OVERNIGHT.md's "test coverage on the new validate/
adjudication paths (they merged TONIGHT — fresh code, thin tests likely)"
category precisely; this module shipped hours ago in PR #68.

**Progress 2026-07-02 (overnight):** landed as
`live_server_serves_the_panel_html_and_reflects_an_applied_label`, same
real-socket/background-thread pattern as the existing `/label` test. Asserts,
in order: `GET /` before any label returns 200/`text/html`/the queue item's
`finding_id`, and does *not* yet contain `"Label: Keep"`; a `POST /label`
applies a `keep` verdict; `GET /` afterward now contains `"Label: Keep"`
(server-side `render_item` reflection, not the client-side JS — proves
`render_live` is re-invoked per request with current `labels`, not a
snapshot taken once at server start); and `GET /index.html` returns the same
reflected content, confirming the documented alias behaves identically.
