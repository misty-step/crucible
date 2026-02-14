# CI Resolution Plan

## Summary
- PR 34 failed in `CASSANDRA` because the configured primary model for testing reviews intermittently returned empty output and triggered the fail-fast path.
- A second model produced output, but the run still ended with `FAIL` from CASSANDRA due to that model's findings against current code.

## TODO

- [x] [CI FIX] Stabilize reviewer model for `CASSANDRA`
  - **Files:** `.github/workflows/cerberus.yml`
  - **Issue:** Primary model `openrouter/qwen/qwen3-max-thinking` returned empty output and exhausted retries.
  - **Fix:** Switch CASSANDRA `model` to `openrouter/moonshotai/kimi-k2.5`.
  - **Verify:** Re-run Cerberus review jobs and confirm CASSANDRA uses direct primary model output path.
  - **Estimate:** 5m

- [ ] [FOLLOW-UP] Address code-quality suggestions from CASSANDRA and related reviewers
  - **Files:** `internal/exec/runner.go`, `internal/exec/mock_runner.go`, `internal/exec/runner_test.go`, `internal/testutil/fixtures_test.go`
  - **Issue:** Remaining findings are code quality and test coverage points (coverage, mock semantics, edge cases, security invariant, buffering/resource limits).
  - **Fix:** Handle as scoped follow-up PR to avoid overloading this CI unblock pass.
  - **Verify:** `go test ./...` and targeted regression tests after code updates.
  - **Estimate:** 1h

- [ ] [FOLLOW-UP] Add follow-up tracking for runtime feedback loop
  - **Files:** AGENTS/CLAUDE process notes
  - **Issue:** Review signals now include many in-scope implementation suggestions across infra and tests.
  - **Fix:** Ensure future PRs include explicit decision for merge-blocking vs follow-up items in response comments and CI plan.
  - **Verify:** Next CI review cycle.
  - **Estimate:** 15m
