# Broaden the deterministic prompt-benchmark grader library

Priority: P2 · Status: done · Estimate: M

## Goal

`PromptExpectation` (`crucible-core/src/spec.rs`) has exactly two deterministic
grader variants — `Exact` and `Contains` (`crucible/src/spec_run.rs:1162-1166`,
`prompt_expectation_passes`). Add at least a `Regex` and a
`CaseInsensitiveContains` (or equivalent normalized-match) variant so a spec
author can express more real rubrics without a Rust PR, following the exact
closed-enum + serde-tagged pattern the two existing variants already use.

## Oracle

- [x] `PromptExpectation` gains ≥2 new variants (e.g. `Regex { pattern }`,
  `CaseInsensitiveContains { value }`), each with a matching arm in
  `prompt_expectation_passes`, `expectation_kind`, and `expectation_value`.
- [x] `crucible validate` and `crucible run` accept specs declaring the new
  variants with no other code changes (they route through the existing
  `PromptExpectation` match arms — confirm by running a fixture spec through
  both).
- [x] Each new variant has unit test coverage in `crucible/src/spec_run.rs`
  (pass and fail cases) plus one fixture spec exercising it end-to-end in
  `crucible/tests/cli.rs` or `crucible/tests/fixtures/specs/`.
- [x] A malformed `Regex` pattern fails with a clear error at spec load/validate
  time, not a panic at grading time.
- [x] `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings
  && cargo test --all` (i.e. `scripts/check.sh`) passes.

## Notes

Live-code-verified 2026-07-01: `crucible-core/src/spec.rs:245` defines
`PromptExpectation` as a 2-variant enum; `crucible/src/spec_run.rs:1162-1180`
is the only consumer. This is the deterministic-grader-breadth item named
directly in `~/.factory-lanes/OVERNIGHT.md`'s crucible focus line ("then
broaden the deterministic grader library") and is pure additive, closed-enum
work with no design/taste call — the shape to copy already exists twice in
the file.

**Why:** OVERNIGHT.md names "broaden the deterministic grader library" as
crucible's explicit third overnight priority (after calibration/adjudication
polish and spec-authoring ergonomics); the prompt-benchmark runner's rubric
vocabulary is the concrete, currently-thin grader library in this repo.

**Progress 2026-07-02 (overnight):** landed. `PromptExpectation` gained
`CaseInsensitiveContains { value }` and `Regex { pattern }`, same closed-enum
+ serde-tagged pattern as `Exact`/`Contains`. New workspace dependency:
`regex` (a real regex engine is the only sane implementation of a `Regex`
rubric — no hand-rolled alternative considered).

`prompt_expectation_passes` changed signature from `bool` to
`anyhow::Result<bool>` so a malformed regex pattern is a propagated error, not
a panic or a silent always-fails. Two enforcement points, both real:
`check_prompt_regexes` (new, `pub(crate)`, shared) precompiles every declared
`Regex` pattern and is called from `run_prompt_benchmark` *before* the
`OpenRouterClient` is even constructed (a bad pattern never spends a real API
call), and from `crucible validate` (`validate.rs`) as a named error
(`runner.corpus.tasks[].expectation.pattern`) so a cold agent catches it
before assembling a runnable corpus at all. Live-verified both refusal points
via the actual binary — `crucible validate` on a spec with `"(unclosed"`
reports `valid: false` naming the task; `crucible run` on the same spec exits
1 with the pattern named in the error, never reaching the credential check.

New fixture `evals/prompt-regex-smoke-v0.json` (a phone-number-format regex
task plus a case-insensitive marker task) exercises both new variants
end-to-end: `crucible validate` reports valid/runnable, and `crucible run`
without `OPENROUTER_API_KEY` reaches the same BYOK credential guard every
other `prompt_benchmark` spec does — proving the regex compiled and dispatch
routed correctly without a live network call in the gate. Unit tests cover
`prompt_expectation_passes` pass/fail for both variants, a malformed-regex
compile-error path, `check_prompt_regexes` naming the offending task among
several, and `run_prompt_benchmark` refusing before any model call (asserted
via the error's full chain, since `anyhow::Error::to_string()` only shows the
outermost context — the inner `regex` compile message needs `{:#}` to
surface).
