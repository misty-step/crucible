# Test coverage: malformed --since/--until timestamp bounds

Priority: P2 · Status: done · Estimate: S

## Goal

`run_store::parse_timestamp_bound` (`crucible/src/run_store.rs:1166-1173`,
merged tonight in the runs-database filter slice) has zero direct unit tests
and no CLI test exercising a malformed `--since`/`--until` value. Add tests
proving it fails cleanly (readable error, exit 1) rather than panicking on
garbage input.

## Oracle

- [x] A `#[cfg(test)]` unit test in `run_store.rs` covers: a valid RFC3339
  timestamp, a valid `YYYY-MM-DD` date, and at least two invalid strings
  (empty string, non-date garbage like `"not-a-date"`) — asserting `Err` with
  a message that names the offending value.
- [x] A `crucible/tests/cli.rs` integration test runs `crucible runs list
  --since garbage` (or `--until`) against a populated ledger and asserts a
  clean non-zero exit with a stderr message, not a panic/backtrace.
- [x] `cargo test --all` passes; no `unwrap()`/panic path is introduced to hit
  the new assertions.

## Notes

Live-code-verified 2026-07-01: `crucible/tests/cli.rs`'s
`runs_list_filters_by_config_model_and_date` (line 955) only exercises valid
`--since`/`--until` values (a far-future/far-past bound), never a malformed
one; `rg 'parse_timestamp_bound|invalid.*timestamp|garbage|malformed'` across
`run_store.rs`/`cli.rs` shows the only references are the function's own
`.context(...)` error string and unrelated malformed-JSON tests elsewhere.
This is exactly the "fresh code merged tonight, thin tests likely" category —
`--since`/`--until` filtering shipped in PR #62 a few hours ago.

**Why:** OVERNIGHT.md names test coverage on the newly-merged run-store/
validate/adjudication paths as a safe overnight category; this is a small,
mechanically verifiable gap in that exact code.

**Progress 2026-07-02 (overnight):** landed. Three new `run_store.rs` unit
tests: RFC3339 and bare-date parse to the same Unix ms value (and a
later-time-of-day RFC3339 parses to a strictly later value, catching a
regression to a date-only truncation bug); an empty string and `"not-a-date"`
both refuse with an error naming the offending value and the accepted
formats. One new `crucible/tests/cli.rs` test
(`runs_list_rejects_a_malformed_since_bound_cleanly`) seeds a real ledger via
`crucible run`, then asserts `crucible runs list --since not-a-date` exits 1
with a stderr message naming the value and containing neither `"panicked"`
nor `"RUST_BACKTRACE"`, plus the same for an empty `--until`.
