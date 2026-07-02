# Broaden the deterministic prompt-benchmark grader library

Priority: P2 · Status: ready · Estimate: M

## Goal

`PromptExpectation` (`crucible-core/src/spec.rs`) has exactly two deterministic
grader variants — `Exact` and `Contains` (`crucible/src/spec_run.rs:1162-1166`,
`prompt_expectation_passes`). Add at least a `Regex` and a
`CaseInsensitiveContains` (or equivalent normalized-match) variant so a spec
author can express more real rubrics without a Rust PR, following the exact
closed-enum + serde-tagged pattern the two existing variants already use.

## Oracle

- [ ] `PromptExpectation` gains ≥2 new variants (e.g. `Regex { pattern }`,
  `CaseInsensitiveContains { value }`), each with a matching arm in
  `prompt_expectation_passes`, `expectation_kind`, and `expectation_value`.
- [ ] `crucible validate` and `crucible run` accept specs declaring the new
  variants with no other code changes (they route through the existing
  `PromptExpectation` match arms — confirm by running a fixture spec through
  both).
- [ ] Each new variant has unit test coverage in `crucible/src/spec_run.rs`
  (pass and fail cases) plus one fixture spec exercising it end-to-end in
  `crucible/tests/cli.rs` or `crucible/tests/fixtures/specs/`.
- [ ] A malformed `Regex` pattern fails with a clear error at spec load/validate
  time, not a panic at grading time.
- [ ] `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings
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
