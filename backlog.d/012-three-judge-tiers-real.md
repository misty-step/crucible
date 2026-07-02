# Make deterministic, agentic, and human judges real

Priority: P0 · Status: in-progress · Estimate: XL (epic)

## Goal

Make Crucible's three judgment tiers executable: deterministic graders run
through the core scorer, agentic judges run as model-native BYOK calls only
behind calibration, and human adjudication writes labels through a working
phone/web panel.

## Oracle

- [ ] Deterministic grader execution is the default floor for benchmark tasks
  and uses the one scorer from `013` where code-review keys are involved.
- [ ] `GraderKind::Agentic` is constructed from a spec, invokes a real model
  judge through the model boundary, and refuses to unlock without a
  `CalibrationRecord` that clears the configured agreement threshold.
- [ ] Human adjudication is a working writeback loop: the phone/web panel writes
  `crucible.label.v1` records, records blindness/latency conditions, and can
  resume a session.
- [ ] A single run can include a hybrid grader mix and report which tiers were
  trusted, diagnostic, or refused.

## Verification System

- Claim: judge tiers are behavior, not schema decoration.
- Falsifier: declaring an agentic or human grader leaves the run unchanged, or a
  judge score is reported as trusted without calibration evidence.
- Driver: fixture benchmark with deterministic-only, agentic-diagnostic, and
  human-label-required tasks.
- Grader: integration tests for refusal/diagnostic/trusted states plus a manual
  phone/web writeback receipt.
- Evidence packet: `RunRecord`, `CalibrationRecord`, label JSONL/database rows,
  and a dashboard/adjudication screenshot or HTML artifact.
- Cadence: every new grader kind or calibration rule.

## Children

1. Deterministic grader dispatch against declared spec graders.
2. ✅ Agentic judge config + BYOK model-call adapter; diagnostic mode first.
3. Calibration unlock: κ/agreement threshold, confusion matrix, model-family
   separation from the generator, and refusal messaging.
4. Minimal human writeback server/panel over the existing judgment queue.
5. Hybrid run reporting: trusted vs diagnostic vs refused tiers.

## Notes

This epic folds the real part of `005` into the broader judge system while
leaving `005` as the UI delivery surface. The operator decision is explicit:
"Three judge tiers real: deterministic graders (exists), agentic judge
(`GraderKind::Agentic` constructed and running — model-native, BYOK), human
adjudication with a WORKING phone/web panel (writeback; kill the CSS-only
buttons)."

Progress 2026-07-01: child 2 landed — `RunnerKind::AgenticJudge` +
`CorpusSpec::AgenticJudge` (`crucible-core/src/spec.rs`) and
`run_agentic_judge`/`run_agentic_judge_with_client`
(`crucible/src/spec_run.rs`) make `GraderKind::Agentic` real end to end: a
declared spec with an `Agentic` grader in `graders.graders` (the runner
refuses without one), a live BYOK OpenRouter judge call via the same
`ModelClient`/`OpenRouterClient` the prompt benchmark runner already used
(generalized `OpenRouterClient::from_credential_env`), and a strict
`VERDICT: PASS`/`VERDICT: FAIL` protocol parsed with no silent guessing on an
ambiguous reply. Judge-gaming guard: an `AgenticJudgeTask` can carry
`expected_pass` + `refuse_on_mismatch` — a canary with a known-bad candidate
that the run refuses outright (no evidence persisted) if the judge rubber-stamps
it. Judge provenance flows through the *existing* prompt-evidence persistence
path in `run_store.rs` (`crucible.agentic_judge_evidence.v1` reuses
`merge_prompt_metadata`, now parameterized by a `config_prefix` so judge runs
get their own `judge:` config-id namespace instead of colliding with
`prompt:` runs) straight into `RunRecord`/`EvaluationCard` — the judge model,
prompt hash, and rubric hash are recorded exactly like a prompt benchmark run,
with zero new provenance plumbing. `evals/agentic-judge-smoke-v0.json` is the
durable fixture (real candidate + canary); `crucible run` against it reaches
the same BYOK credential guard as the prompt benchmark runner (proven by CLI
test, no live model call in the gate). Remaining: children 3 (real
`CalibrationRecord` agreement-threshold unlock — this slice's canary is a
binary refuse/pass tripwire, not a measured κ/agreement gate), 4 (human
writeback), 5 (hybrid trusted/diagnostic/refused run reporting across all
three tiers).

Also fixed in this slice (found while gating it): `scripts/leak-scan.sh`'s
gitleaks `dir` pass was silently broken — `gitleaks dir` only accepts a single
positional path, and handing it the whole tracked-file list as separate argv
words doesn't error; past some combined-argv-length threshold it silently
joined them into one bogus path, so the gate was quietly running grep-floor-only
coverage (a false "clean") until this session's 95th tracked file tipped the
joined string over the OS's `ENAMETOOLONG` limit and the gate started failing
on unrelated `target/` noise instead. Fixed to loop `gitleaks dir` one tracked
file at a time (~3.5s locally for 95 files).
