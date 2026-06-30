# Code-review eval: the first benchmark Crucible owns end to end

Priority: P1 · Status: in-progress · Estimate: XL (epic)

## Goal

Prove Crucible's model on one real family: define, calibrate, and iterate the
agentic code-review eval; industrialize its adjudication; bootstrap labels for
real diffs; and emit a Harbor benchmark Daedalus can re-score and optimize
against.

## Oracle

- [x] One real disputed Cerberus finding set is adjudicated through Crucible's
  CLI (the five labels) and exported as a Harbor `adjudications.md`
  key-extension + `tests/expected.json` defect that Daedalus re-scores against —
  **round-trip lead-verified 2026-06-30: an accepted finding flips FP→TP, reward
  0.8→1.0 via `daedalus-score`** (see Delivered 2026-06-30).
- [ ] At least one currently-blocked Daedalus arena
  (`pr-review-{simplification,product,verification}`) gains ≥5 labeled,
  calibrated Harbor tasks.
- [ ] Any model/agentic judge ships with a measured judge-vs-human agreement
  (Cohen's κ) and is gated, not assumed.
- [x] Per-config code-review rates are reported with a Wilson interval; paired
  (McNemar) comparison + noise-floor verdict now landed (epic 003).

## Children (ordered)

1. ✅ **(SPIKE — gating) DONE — GO (2026-06-29).** See "Spike result" below.
2. ✅ **Corpus + adapter (002.2) — DONE 2026-06-29.** Cerberus `ReviewArtifact`
   → Daedalus key projection, tested against the real fixture.
3. ✅ **Deterministic pre-graders (002.3) — DONE 2026-06-29.** `schema_valid`,
   `dedup`, `key_match` (file+line±tol, normalized category), `recoverable_misses`.
4. ✅ **Finding-judgment record + CLI adjudication queue (002.4) — DONE 2026-06-30.**
   `crucible adjudicate` builds an ordered `JudgmentQueue` from a grade and applies
   Verdict+Disposition into append-only, reconciled `Label`s (latest-wins);
   `Finding.id` threads through the adapter as `source_id` so labels trace to source.
5. ✅ **Export (002.5) — DONE 2026-06-30.** `crucible export` writes the
   `adjudications.md` log AND the `tests/expected.json` defect Daedalus actually
   scores (the round-trip CLOSES). Calibration record + κ-gated judge still open
   (needs human labels). **Governance: RATIFIED.**
6. **Borrowed/agentic model-judge behind the calibration gate.** Phone UI = 005.
   Blocked on real human labels to calibrate against (κ gate).

## Delivered (2026-06-29) — deterministic core, shipped

A subagent workflow (build → thermonuclear review → QA → refactor) built and
shipped the dependency-free deterministic core. Evidence: `scripts/check.sh`
green; `crucible grade --json` drives clean over the real Cerberus artifact;
all real Daedalus keys load (was 24/48 before the critical fidelity fix).

Landed: `crucible-core` (artifact/Finding types, `adapter` 002.2, `grade` 002.3,
`measure` — Wilson/proportion/agreement, first primitives of 003); `crucible`
CLI `adapt`+`grade --json` (partial 006.1); repo gate `scripts/check.sh`
(006.2 ✅) + AGENTS.md gate section.

## Delivered (2026-06-30) — adjudication queue + Daedalus round-trip, shipped

A three-workflow agent pipeline (deliver → thermonuclear review+QA → fix/refactor)
landed 002.4 + 002.5 plus the rigor (003) and type (004) substrate. The
thermonuclear review (38 agents, each finding refuted by 2 skeptics + 4 QA lanes
on real data) caught a **blocking contract error every build agent missed**:
Daedalus's scorer reads `tests/expected.json` (span-based `defects[]`), NOT the
`solution/findings.json` Crucible was extending — so the original export re-scored
an accepted finding as a *false positive* (reward 1.0→0.8, the inverse of the ACCEPT
doctrine). Fixed: `export --expected` now extends the real scorer key; **lead-verified
round-trip via `daedalus-score`: accepted finding flips FP→TP, reward 0.8→1.0, FP
1→0.**

Also landed: label reconciliation (append-only correction = latest-wins; no double
key-extension or version double-bump), CLI fail-fast + full-field escaping + stable
exit codes 0/1/2, and the McNemar p>1 fix (the noise-floor gate). Gate green
(fmt/clippy -D warnings/test/build/leak-scan/cargo doc -D warnings); 48/48 real
Daedalus keys load.

Residual (tracked → **008**): the export span is a single-line under-approximation
(`line_start==line_end==line`); `crucible grade`'s matcher is a category-strict
pre-adjudication floor, not Daedalus's full span+FP+severity reward. Authoritative
scoring stays with `daedalus-score`.

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
  --head <ref> --json` emits `ReviewArtifact { schema_version, findings: [Finding] }`.
  Real artifact at `cerberus/evidence/self-review-001/artifact.json`.
- **Daedalus corpus + adjudication — confirmed.** `arenas/pr-review-*`;
  `adjudications.md` (ACCEPT→key+version bump / OUT-OF-SCOPE); `holdout-ledger.md`;
  `solution/findings.json` key (severity OPTIONAL). **Correction (2026-06-30): the
  machine scorer reads `tests/expected.json` (`{defects:[{id,file,line_start,
  line_end,category,note?}]}`), not `solution/findings.json` — see Delivered.**
- **Scorer is Rust** (`daedalus/crates/daedalus-core/src/score.rs`).

Governance: **RATIFIED by operator 2026-06-29 — Crucible is authorized to author
arena versions / write `adjudications.md`** (unblocks 002.5 export + epic 007).

## Notes

Scope guard: Crucible designs and calibrates the measurement; it does NOT run the
optimization search loop — that is Daedalus. Draws uncertainty/calibration
primitives from epic 003 and types from epic 004.
