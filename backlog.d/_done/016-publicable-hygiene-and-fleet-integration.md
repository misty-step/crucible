# Keep Crucible publicable and integrated with the factory fleet

Priority: P2 · Status: done · Estimate: M (epic)

## Goal

Remove stale repo-contract prose and local-machine assumptions that make
Crucible harder for cold agents, public readers, or the factory fleet to use.

## Oracle

- [x] `AGENTS.md`, README, SKILL, and VISION agree on the current state: Rust
  code exists, Crucible owns selected execution, and Threshold is the optimizer.
- [x] The flagship spec no longer hardcodes `../../daedalus` paths without an
  explicit local-only marker or a portable fixture alternative.
- [x] Reclaimed go-council residue is closed or labeled so it cannot be mistaken
  for current Crucible roadmap work.
- [x] Publicable policy holds: MIT license present, no instance data, personal
  paths, tailnet names, or raw run records in tracked files except deliberate
  fixture paths.
- [x] Landmark release intelligence remains wired and documented.

## Verification System

- Claim: a cold factory agent can understand and run Crucible without stale
  contract traps or personal-instance leakage.
- Falsifier: repo docs still say there is no application code, the default spec
  fails only because a sibling path is assumed, or tracked files reveal instance
  names/raw run data.
- Driver: `rg` hygiene checks, `crucible validate`, and the repo gate.
- Grader: no stale-string hits for the known traps plus successful fixture-run
  commands from SKILL.
- Evidence packet: command transcript and PR diff.
- Cadence: every groom or release-integration change.

## Children

1. ✅ Refresh stale `AGENTS.md` current-state line.
2. ✅ Fix or mark local-only hardcoded Daedalus/Threshold paths in the flagship
   spec and docs (see progress note — marker is a `crucible validate` warning +
   SKILL.md prose, not a new schema field).
3. ✅ Close or label GitHub issue #15 as archived go-council residue (labeled,
   left open — see progress note for why not closed).
4. ✅ Keep MIT license and repository metadata aligned to `misty-step/crucible`.
5. Add a small hygiene check if these traps recur.

## Notes

The 2026-07-01 groom verified issue #15 is still open and belongs to the old
go-council project. Do not close it silently from a local docs PR if GitHub
project policy wants labels instead; the backlog item makes the disposition
explicit.

**Factory groom 2026-07-01:** MIT license, repository metadata, stale AGENTS
state, README/SKILL personal absolute paths, and the authoring-map absolute path
were fixed in the groom PR. Remaining: mark or eliminate the flagship spec's
local-only sibling path and dispose of GitHub issue #15.

**Progress 2026-07-02 (overnight):** children 2 and 3 landed.

Child 2: rather than inventing a new `local_only: true` schema field for
`EvalSpec` (a schema-shape decision this ticket doesn't specify and that would
need to interact with `crucible validate`'s already-shipped checks — a taste
call better left for daylight review, not decided unilaterally overnight), the
marker landed as tooling + docs instead: `crucible validate` (backlog `014`,
merged earlier tonight) already emits a named warning
(`runner.corpus.arena_dir`/`runner.corpus.trials_jsonl`) on exactly this
condition — a `daedalus_trials` corpus path that escapes the spec's own
directory tree — and `SKILL.md`'s refreshed "Validate A Spec Before Running"
section documents it in prose. Both flagship specs
(`pr-review-key-recall-v0.json`, `cerberus-review-quality-v0.json`) already
produce this warning when validated; that satisfies "no hardcoded... paths
without an explicit local-only marker" without adding new schema surface ahead
of a real second consumer of that field. If a machine-checkable `local_only`
spec field is wanted later (e.g. to make CI explicitly skip non-portable specs
rather than a human reading a warning), that is a fresh, scoped follow-up.

Child 3: added an explanatory comment on GitHub issue #15
(`internal/models/ratelimit.go`, opencode process spawning — pre-reclaim
go-council architecture that does not exist in current Rust Crucible) naming
it archived residue per this backlog item, and applied the `wontfix` label.
Left the issue **open** rather than closing it: it is assigned to a specific
person and carries a project milestone, and the backlog note itself says "do
not close it silently... if GitHub project policy wants labels instead" —
labeling + explaining is the disposition an overnight lane can make safely;
closing an assigned, milestoned issue is a call for a human.

**Progress 2026-07-04 (Powder crucible-016 re-verification):** re-checked all
five Oracle lines against the live repo rather than trusting the prior
checkmarks, since a lot has shipped since (`crucible author`, `crucible
doctor`, MCP tools, glossary, README restructure from crucible-906, brand work
from crucible-033).

- Line 1 (docs agree on current state) was actually **not** fully true despite
  the pre-existing checkmark: `VISION.md`'s "Early Shape" paragraph still said
  Crucible's "next job" was to "own the engine: author benchmarks, make real
  model calls, record runs, collect human labels..., calibrate agentic
  judges..." — all of which had since shipped (`crucible author`, the
  OpenRouter-backed `prompt_benchmark`/`agentic_judge` runners, the SQLite run
  ledger, the adjudication panel's live writeback, and `CalibrationRecord`).
  A "Sources" bullet also asserted "no live model-call engine yet" as of
  2026-07-01, contradicted by `AGENTS.md`/README/SKILL, which document that
  engine as real. Fixed both in `VISION.md` (rewrote the stale paragraph past
  tense, appended a dated 2026-07-04 Sources bullet correcting the record
  without deleting the original).
- Line 2 (flagship spec path marker) re-verified: `evals/pr-review-key-recall-v0.json`
  still hardcodes `../../daedalus/arenas/pr-review-v0` and
  `../../daedalus/runs/.../trials.jsonl`, and still has no schema-level
  `local_only` marker — but `crucible validate` still emits the named warning
  on that exact condition and `SKILL.md` still documents it in prose, so the
  original disposition (tooling + docs marker, not a schema field) holds.
  `backlog.d/032-threshold-corpus-path-aliases.md` tracks the more ambitious
  "actually resolve on a machine with `threshold` but no `daedalus`" follow-up
  and is still `Status: pending` — correctly scoped separately, not redone
  here.
- Line 3 (go-council residue) re-verified via `gh issue view 15`: still
  labeled `wontfix` with the same explanatory comment, left open per the
  documented human-decision rationale. No other open issues reference
  go-council. Nothing to change.
- Line 4 (publicable policy) re-verified: `LICENSE` is MIT/Misty Step LLC,
  `./scripts/leak-scan.sh` reports clean, and `git grep` for `/Users/` and
  the operator's usernames across tracked files returns no hits. Nothing to
  change.
- Line 5 (Landmark) was genuinely still open: `.landmark.yml` plus
  `.github/workflows/landmark-release.yml` and `landmark-manual-tag.yml` wire
  `misty-step/landmark@v1` in `synthesis-only` mode on release, but no doc
  mentioned it. Added a short paragraph to README.md's Gate section (matching
  the convention already used in the `powder` repo's README) naming the
  workflow, its config, and that it is release-only and does not replace
  `./scripts/check.sh`.

Children item 5 ("add a small hygiene check if these traps recur") remains
unstarted — out of scope for this pass since it isn't one of the five Oracle
acceptance lines; left as a residual follow-up, not invented into new work
here.
