# Code-review eval: the first benchmark Crucible owns end to end

Priority: P1 · Status: in-progress · Estimate: XL (epic)

## Goal

Prove Crucible's model on one real family: define, calibrate, and iterate the
agentic code-review eval; industrialize its adjudication; bootstrap labels for
real diffs; and emit a Harbor benchmark Daedalus can re-score and optimize
against.

## Oracle

- [ ] One real disputed Cerberus finding set is adjudicated through Crucible's
  CLI (the five labels) and exported as a Harbor `adjudications.md`
  key-extension / arena version bump that Daedalus re-scores against.
- [ ] At least one currently-blocked Daedalus arena
  (`pr-review-{simplification,product,verification}`) gains ≥5 labeled,
  calibrated Harbor tasks.
- [ ] Any model/agentic judge ships with a measured judge-vs-human agreement
  (Cohen's κ) and is gated, not assumed.
- [x] Per-config code-review rates are reported with a Wilson interval; paired
  (McNemar) comparison + noise-floor verdict still TODO (epic 003).

## Children (ordered)

1. ✅ **(SPIKE — gating) DONE — GO (2026-06-29).** See "Spike result" below.
2. ✅ **Corpus + adapter (002.2) — DONE 2026-06-29.** Cerberus `ReviewArtifact`
   → Daedalus key projection, tested against the real fixture. (Harbor
   task-directory *round-trip* write still TODO — see 002.5.)
3. ✅ **Deterministic pre-graders (002.3) — DONE 2026-06-29.** `schema_valid`,
   `dedup`, `key_match` (file+line±tol, normalized category), `recoverable_misses`.
4. **Finding-judgment record + CLI adjudication queue** — five labels
   (correct/important/duplicate/actionable/noise) + rationale; emits the
   judgment-queue artifact contract. Verdict/Disposition types scaffolded
   (`crucible-core/src/adjudication.rs`); interactive queue is the **active next**.
   Prereq follow-up: carry `Finding.id` through the adapter so labels trace to
   the source finding.
5. **Export** — `adjudications.md` key-extension (ACCEPT) or new Harbor task;
   calibration record (judge vs human κ); scored report. **Governance: RATIFIED.**
6. **Borrowed/agentic model-judge behind the calibration gate.** Phone UI = 005.

## Delivered (2026-06-29) — deterministic core, shipped

A subagent workflow (build → thermonuclear review → QA → refactor) built and
shipped the dependency-free deterministic core. Evidence: `scripts/check.sh`
green (92 tests, fmt/clippy -D warnings/test/build); `crucible grade --json`
drives clean over the real Cerberus artifact across matched/disputed/missed;
**all 42 real Daedalus keys load (was 24/48 before the critical fidelity fix).**

Landed: `crucible-core` (artifact/Finding types, `adapter` 002.2, `grade` 002.3,
`measure` — Wilson/proportion/agreement, first primitives of 003); `crucible`
CLI `adapt`+`grade --json` (partial 006.1); repo gate `scripts/check.sh`
(006.2 ✅) + AGENTS.md gate section.

Thermonuclear review: 19 findings, 4 blocking — all fixed pre-ship (line-0 match
inflation; `KeyFinding.severity` required vs half the real keys; exact category
over-penalizing cross-vocab → `recoverable_misses`; CLI test gaps).

Tracked non-blocking follow-ups:
- Adapter drops `Finding.id` → matched/disputed can't trace to source; needed
  for the adjudication evidence packet (gates 002.4). [this epic]
- Category-match altitude: decide final predicate (file+line vs +category) with
  operator; `recoverable_misses` currently surfaces colocated cross-vocab. [this epic]
- Measurement hardening (wilson successes>n guard; agreement length-mismatch →
  Option; n==0 `--json` null vs 0.0; surface dropped-invalid count) → **epic 003**.
- Secret/content-leak scan in the gate → **epic 006**.

## Verification System

- Claim: Crucible turns raw Cerberus findings over real/disputed diffs into
  adjudicated, calibrated, Daedalus-importable benchmark tasks.
- Falsifier: the round-trip fails — Daedalus cannot re-score the emitted Harbor
  artifact, or the judge ships uncalibrated, or a "winner" is declared inside the CI.
- Driver: `crucible` CLI over a real Daedalus disputed-finding record +
  `cerberus review-diff --base --head --json`.
- Grader: deterministic (anchor cites a real changed line; dedup; key-match) +
  calibrated model-judge (κ-gated) + human adjudication (five labels).
- Evidence packet: emitted Harbor task dir + `adjudications.md` diff +
  calibration record (κ, confusion matrix) + scored report with CIs.
- Cadence: per child; re-run on each arena version bump.

## Spike result (002.1 — 2026-06-29): GO

Verified read-only against live `cerberus/` and `daedalus/`:

- **Cerberus headless JSON — confirmed.** `cerberus review-diff --base <ref>
  --head <ref> --json` emits `ReviewArtifact { schema_version, findings: [Finding] }`
  (`cerberus/src/main.rs:467`, `src/schema.rs:235-337`). Real artifact at
  `cerberus/evidence/self-review-001/artifact.json`.
- **Daedalus corpus + adjudication — confirmed.** `arenas/pr-review-*`;
  `adjudications.md` (ACCEPT→key+version bump / OUT-OF-SCOPE); `holdout-ledger.md`;
  `solution/findings.json` key format `{findings:[{file,line,category,severity?,description}]}`
  (severity OPTIONAL — present in only 14/48 keys).
- **Scorer is Rust** (`daedalus/crates/daedalus-core/src/score.rs`).

Adapter contract (delivered): INPUT Cerberus `ReviewArtifact.findings[]`; OUTPUT
Daedalus key rows; mapping anchors→file/line, severity enum→vocab, category/desc.

Governance: **RATIFIED by operator 2026-06-29 — Crucible is authorized to author
arena versions / write `adjudications.md`** (unblocks 002.5 export + epic 007).
The five labels also capture "correct but out-of-contract" (ADJ-2).

Residual for 002.4/002.5: locate one concrete `runs/<id>/` disputed-finding
record (`20260623T…` / `20260625T…`) to drive the first adjudication round-trip.

## Notes

Scope guard: Crucible designs and calibrates the measurement; it does NOT run the
optimization search loop — that is Daedalus. Draws uncertainty/calibration
primitives from epic 003 and types from epic 004.
