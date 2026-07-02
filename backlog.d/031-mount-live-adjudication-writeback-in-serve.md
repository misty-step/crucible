# Mount live adjudication writeback inside `crucible serve`

Priority: P1 · Status: pending · Estimate: M

## Goal

`crucible serve` now links/render-composes existing adjudication panel artifacts,
but it does not mount the `adjudication_server` writeback loop inside the main
application shell. The operator can reach the existing panel, yet label commits
still require running `crucible adjudication-panel --serve` separately when real
writeback is desired.

## Oracle

- [ ] A run with a `crucible.judgment_queue.v1` artifact can be opened from
  the `Adjudicate` view inside `crucible serve` and can commit Keep/Nit/Wrong/
  Noise labels without starting a second server.
- [ ] The write path still mints labels through `apply_label` and persists the
  same `crucible.label.v1` array shape that `adjudicate --apply` already reads.
- [ ] Existing `adjudication-panel --serve` behavior remains intact; `serve`
  composes the same core path rather than forking a second label model.
- [ ] A CLI/integration test drives the mounted HTTP route over a real
  `TcpStream` and proves a committed label lands on disk.

## Notes

UI lane 2026-07-02 deliberately stopped at renderer-backed links for v1 because
the main `serve` HTTP loop is synchronous and local-first. The next slice should
factor the reusable label handler out of `adjudication_server.rs` or expose a
small shared writeback function, then mount it under the app route.
