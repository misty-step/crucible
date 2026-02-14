# CI Resolution Plan

## Goal

Make `CASSANDRA` deterministic and avoid CI-blocking parse-failures from unstructured model output.

## Planned actions

- [x] [CI FIX] Pin `CASSANDRA` reviewer to a non-thinking model with stable structured-output behavior.
  - **Files**: `.github/workflows/cerberus.yml`
  - **Change**: `openrouter/qwen/qwen3-max-thinking` → `openrouter/moonshotai/kimi-k2.5`
  - **Rationale**: The observed failure was tied to unstructured output from the previous model; a model profile that already passes successfully in other council roles reduces parser variance.
  - **Estimate**: 10m

- [x] [CI FIX] Keep parse/output strictness intact to preserve signal quality.
  - **Files**: `.github/workflows/cerberus.yml`
  - **Change**: no parser logic changes in this repo (handled by upstream `cerberus`).
  - **Rationale**: this keeps review quality control centralized in `misty-step/cerberus`.
  - **Estimate**: 10m

## Verification

- [ ] Re-run the active PR check and confirm:
  - `CASSANDRA` step no longer posts `parse-review: no ```json block found`
  - no `testing review verdict: FAIL`
  - council aggregate is not blocked by `CASSANDRA`

## Follow-up

- If unstructured output recurs with the new model, switch `CASSANDRA` to a separate review job-level `continue-on-error` policy so review results are still captured without blocking merge on formatter variance.
