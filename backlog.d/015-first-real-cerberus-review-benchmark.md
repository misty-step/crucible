# Ship the first real Cerberus review-quality benchmark

Priority: P1 · Status: in-progress · Estimate: XL (epic)

## Goal

Create the first production-grade Crucible benchmark: Cerberus review quality,
measuring pass^k consistency and key-recall against adjudicated truth so
Cerberus can earn its path from advisory toward blocking.

## Oracle

- [ ] The benchmark runs Cerberus-style review outputs against an adjudicated
  code-review truth set and reports key-recall with defensible intervals.
- [ ] pass^k consistency is measured across repeated runs/configs, with a
  confidence interval and a noise-floor verdict.
- [ ] The benchmark uses human-adjudicated labels for at least one slice and
  records which judge tiers were trusted vs diagnostic.
- [ ] Results export in a form Cerberus and Threshold can consume for improvement
  loops without turning Cerberus into a repo-level merge gate.

## Verification System

- Claim: Crucible can measure Cerberus review quality on real review tasks.
- Falsifier: the benchmark only replays old Threshold scores, lacks adjudicated
  truth, or cannot distinguish repeated-run consistency from noise.
- Driver: Crucible-authored benchmark over Cerberus artifacts and Harbor scorer
  keys.
- Grader: one-scorer deterministic key recall, human labels, and calibrated
  agentic judge only after `012` unlocks it.
- Evidence packet: benchmark spec, run records, labels, pass^k report, dashboard
  view, and export bundle.
- Cadence: per benchmark version.

## Children

1. ✅ Define the benchmark spec and task set from current Cerberus/Threshold data.
2. Collect/adapt adjudicated truth through the human queue (partial — uses the
   arena's existing frozen `tests/expected.json` keys, not a fresh human
   adjudication pass; see progress note).
3. ✅ Run repeated Cerberus configs and compute pass^k consistency (k=5, one
   candidate — see progress note for multi-config scope left).
4. ✅ Report key-recall, intervals, and pass^k for the first live run (partial —
   no noise-floor verdict, cost breakdown, or worked examples yet; see note).
5. Export benchmark/run artifacts to Cerberus and Threshold consumers.

## Notes

Operator decision 2026-07-01: "First real benchmark: cerberus review quality
(pass^k consistency + key-recall vs adjudicated truth) — the eval that gates
cerberus's path to blocking." Until this benchmark is defensible, Cerberus stays
advisory everywhere.

Progress 2026-07-01: first live scored run landed. `evals/cerberus-review-quality-v0.json`
(the durable spec, authored and run through Crucible's own CLI — `crucible run`,
no bespoke script) points the existing `key_recall`/`daedalus_trials` runner at
real data: the `incumbent` candidate (Cerberus's production reviewer config,
`deepseek/deepseek-v4-pro`) from Threshold's real
`../../daedalus/runs/20260625T161856Z-search-cerberus-reviewer/trials.jsonl`
search run, 5 trials × 6 tasks, scored against the live `pr-review-v0` arena
(`../../daedalus/arenas/pr-review-v0`, version `0.3.0`) — the same hardcoded
sibling-checkout convention the existing flagship spec
(`evals/pr-review-key-recall-v0.json`) already uses (backlog `016`'s hygiene
item about that convention is still open, not solved by this ticket).

This landed a real `spec_run.rs` feature, not just the spec: `compute_pass_k`
(new, unit-tested) groups a run's task results by `task_id`, requires every
task to share one trial count `k ≥ 2` (else refuses to report a number, per
"never report a rate you cannot defend"), and Wilson-scores the fraction of
tasks where *every* trial fully matched the key (zero missed, zero false
positives) — reusing `crucible_core::Leaderboard`'s `solve_rate` "task is the
independence unit" pattern, computed from Crucible's own re-graded key match
(`crucible_core::score_against_expected_key`), not the trial's self-reported
`reward`/`recall` fields. Both the `key_recall`/`daedalus_trials` and
`cerberus_receipt_bundles` runners now carry an optional `pass_k` field on
`crucible.spec_run_evidence.v1` (`None` for receipt-bundle runs — one artifact
per task, no repetition to measure).

**The first live scored run** (`crucible run evals/cerberus-review-quality-v0.json
--db <db>`, exit 0, persisted and queryable via `crucible runs list/show
--benchmark cerberus-review-quality-v0`):

- `pr_review_key_recall`: 23/45 matched, point 0.511, 95% Wilson CI
  [0.370, 0.650].
- `pass^5`: 2/6 tasks fully matched the key on **every** trial
  (`js-clean-rename`, `py-pagination`), point 0.333, 95% Wilson CI
  [0.097, 0.700]. `py-auth-sqli` and `py-file-cache` never fully matched
  across 5 trials; `rs-retry-backoff` swung from a fully-correct trial to a
  fully-missed one — real, measured inconsistency in the production config.

**Open discrepancy, not resolved here**: `026-consistency-floor.md` in the
Cerberus repo cites a prior pass^5 measurement "near 0.0434" as the number to
beat; this run measured 0.333 on the same arena/candidate concept. The two
numbers are **not directly comparable as reported** — this run uses Crucible's
own re-graded key match against the *current* (`0.3.0`) adjudicated keys over
exactly 6 tasks / 30 trials from one specific Threshold search run, while the
0.0434 figure's source data, task set, and scoring method are not established
here (it may span more tasks, other candidates, an older arena version, or a
different pass/fail bar). Reconciling the two — same corpus, same scorer, same
definition of "passed" — is real follow-up work before either number can gate
anything; flagging the gap explicitly rather than either dismissing 0.0434 or
uncritically treating 0.333 as the new floor.

Remaining, in rough priority order: reconcile the pass^5 discrepancy above;
extend the spec/corpus to cover Threshold's other search runs and candidates
(a true multi-config pass^k comparison, not just `incumbent`); a real
noise-floor verdict on the pass^k delta (this epic's `PairedComparison`/
`DeltaVerdict` kernel already exists in `crucible-core`, just not wired to
this benchmark yet); cost/latency reporting per task; human-adjudicated
truth for at least one slice (today this run trusts the arena's existing
frozen keys, not a fresh Crucible-owned adjudication pass); and the
Cerberus/Threshold export bundle (child 5).

**Reconciliation 2026-07-02 (overnight, backlog `024`):** the 0.0434 figure's
full provenance is traced with confidence. It comes from
`~/.factory-lanes/groom/cerberus.md` §9, which cites Threshold/Daedalus arena
run `20260623T183514Z-search-cerberus-reviewer`
(`report.md`'s "Reliability (pass rate at reward ≥ 1.00)" table): candidate
`seed2-kimi-k2-7-code-trace-callers`, n=30 (6 `pr-review-v0` tasks × 5
trials), pass≥1.00 = 0.5667, **pass^5 = 0.0434** — computed by Daedalus's own
search/reliability scorer from each trial's self-reported `reward` field
against a reward ≥ 1.00 floor.

This is a different number on every axis from Crucible's 0.333:

1. **Different candidate.** 0.0434 scores `seed2-kimi-k2-7-code-trace-callers`
   — a search-discovered agent composition, one of several candidates a
   Threshold optimizer run was evaluating. 0.333 scores `incumbent`
   (`deepseek/deepseek-v4-pro`) — Cerberus's actual shipped production
   reviewer config. These are not the same reviewer. Notably,
   `seed2-kimi-k2-7-code-trace-callers` never appears as `incumbent` in any
   Daedalus run inspected, and `incumbent` never gets its own pass^5 row in
   any Daedalus `report.md` — Crucible's 0.333 is the *first* pass^5 ever
   computed for the production config specifically.
2. **Different source run.** 0.0434's trials are from
   `20260623T183514Z-search-cerberus-reviewer`; 0.333's trials are from
   `20260625T161856Z-search-cerberus-reviewer` (per this ticket's own Notes,
   above) — a separate Threshold search run two days later. Both runs cover
   the same 6 `pr-review-v0` task ids (`js-cart-total`, `js-clean-rename`,
   `py-auth-sqli`, `py-file-cache`, `py-pagination`, `rs-retry-backoff`,
   confirmed by reading both runs' `trials.jsonl`), and the arena's
   `adjudications.md` (the file that drives version bumps) has been
   unchanged since 2026-06-10 — well before both runs — so both used the
   same frozen `0.3.0` keys. Arena/key version is **not** a scope
   difference; candidate identity and source run are.
3. **Different scorer.** 0.0434 is Daedalus's own reward-based pass rate
   (`reward ≥ 1.00`, self-reported by the trial). 0.333 is Crucible's
   independent re-graded key-recall match (`score_against_expected_key`)
   against the current adjudicated keys — Crucible never reads the trial's
   `reward` field for this benchmark. Even if the two numbers scored the
   same candidate and run, they would not be measuring the same pass/fail
   predicate.
4. **Bonus finding, not previously visible:** the *same* candidate name,
   `seed2-kimi-k2-7-code-trace-callers`, scores pass^5 = 0.0434 in the
   06-23 run and pass^5 = 0.1088 in the 06-25 run — a >2x swing from
   Daedalus's own scorer alone, before Crucible's independent regrading
   enters at all. At n=30 (6 tasks × 5 trials), pass^5's sampling variance
   is large regardless of scorer or candidate — consistent with Crucible's
   own wide Wilson CI on 0.333, `[0.097, 0.700]`.

**Conclusion:** the two numbers are not comparable and neither should gate
anything as currently measured — not because either is wrong, but because
they describe different reviewers, different runs, and different scoring
methods. No number is picked as "correct" here; that stays an operator call.
**Named next step:** to get a true apples-to-apples pass^5 for `incumbent`
against Daedalus's own reward-based scoring — or a true apples-to-apples
pass^5 for `seed2-kimi-k2-7-code-trace-callers` against Crucible's
re-graded key-recall scoring — someone needs to author a Crucible spec
pointing at the other run/candidate combination and run it; both are
mechanically straightforward given the existing `cerberus-review-quality-v0`
spec shape (backlog 017's grader library and this epic's runner already
generalize past `incumbent`), but authoring and running that spec is real
work, not a documentation-only task, and is left for whoever picks up
"extend the spec/corpus to cover Threshold's other search runs and
candidates" above.
