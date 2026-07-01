# Make deterministic, agentic, and human judges real

Priority: P0 · Status: ready · Estimate: XL (epic)

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
2. Agentic judge config + BYOK model-call adapter; diagnostic mode first.
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
