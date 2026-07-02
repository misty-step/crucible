# Make specs honest and expose agent-first benchmark controls

Priority: P1 · Status: in-progress · Estimate: L (epic)

## Goal

Expose CLI and MCP surfaces to define, validate, manage, and run benchmarks for
arbitrary model configs, while making every spec field either wired into
execution or rejected by validation.

## Oracle

- [ ] `crucible validate <spec>` reports schema, portability, grader, baseline,
  fixture, confidence, and runner support errors with stable JSON and exit codes.
- [ ] A spec that declares unsupported graders, baselines, fixtures, or
  confidence behavior refuses to run instead of silently ignoring them.
- [ ] Agents can define/manage/run benchmarks through CLI and MCP without
  editing Rust for supported runner families.
- [ ] The four decorative fields identified by the groom report (`graders`,
  `fixtures`, `baselines`, `uncertainty.confidence`) are wired or removed.

## Verification System

- Claim: spec files are executable contracts, not aspirational metadata.
- Falsifier: changing a declared grader/baseline/confidence field has no effect
  and does not fail validation.
- Driver: positive fixture specs and negative fixture specs for each unsupported
  field.
- Grader: CLI/MCP integration tests asserting exact validation failures and run
  refusals.
- Evidence packet: validation JSON snapshots and MCP tool-call transcripts.
- Cadence: every spec schema or runner-kind change.

## Children

1. ✅ `crucible validate` CLI with stable JSON and human output.
2. ✅ Validation rules for portability: no hardcoded sibling paths in flagship
   specs unless explicitly marked local-only (implemented as a warning, not a
   hard error — see progress note on why).
3. Validation rules for honest fields: graders, confidence, aggregation,
   runner kind ✅; fixtures already honest (no work needed — see note);
   baselines still genuinely unenforced (reported as a warning, not wired).
4. MCP tools for validate ✅ (list/show/compare already existed pre-epic); no
   create/update — this repo's specs are files, not a store (see note).
5. ✅ Fixture-backed authoring smoke: a cold agent creates a tiny benchmark
   that validates and runs hermetically — see
   `backlog.d/022-cold-agent-authoring-smoke-test.md`.

## Notes

This is the agent-first surface from the operator overlay. Do not build a broad
platform ahead of `010`; make the controls thin over the real engine and typed
spec contract.

**Progress 2026-07-02 (overnight):** children 1, 2 (as a warning), 3 (three of
four fields), and 4 (validate only — see below) landed.

`crucible validate <spec>` / MCP `crucible_validate` reports `{valid,
runnable, errors, warnings}` for a declared spec, without needing a runnable
corpus (no sibling checkout, no trials file, no `OPENROUTER_API_KEY`) — the
whole point of validating *before* running. It calls the exact same
`preflight_spec` function `crucible run` calls to decide whether to refuse, so
the two can never drift into "validate says fine, run refuses" or vice versa.
Like every other subcommand, `validate` exits 0 whether or not the spec is
valid (the verdict is in the body) and exit 1 only on a genuine load error
(unknown schema, malformed JSON) — the same discipline `grade`/`adjudicate`
already use.

`preflight_spec` — one function, shared by all three runners, replacing three
copies of the same bail logic — now enforces, for real, all three of the
oracle's "wired or removed" fields that were still decorative going into this
slice:
- `aggregation`/`uncertainty.method` — already enforced pre-epic, unchanged.
- `uncertainty.confidence` — **newly enforced**. The runner has only ever
  computed a hardcoded 95% Wilson interval (`main.rs`'s `Z_95 = 1.96`); a spec
  declaring any other value now refuses to run instead of the value being
  silently ignored.
- `graders` — **newly enforced for `key_recall` and `prompt_benchmark`**
  (both now require a declared `Deterministic` grader — the tier they
  actually execute); `agentic_judge`'s existing Agentic-grader requirement
  (landed in the agentic-judge-tier slice) is unchanged, just refactored into
  the same shared function. All five real committed specs already declared
  the right grader kind, so this is zero-regression — verified live against
  every file in `evals/` and the CLI/CI fixtures.

`fixtures` turned out to already be honest going into this epic — it was
flagged decorative by the 2026-07-01 groom, but `run_store.rs`'s
`declared_fixture_refs` (landed with the runs-database work) already reads
`spec.fixtures` into `EvaluationCard.provenance.fixture_refs`. No code
changed for this field in this slice; `validate` does not flag it because
there is nothing dishonest left to flag.

`baselines` is **not** wired to a hard refusal in this slice — deliberately.
The real, currently-working flagship spec (`evals/pr-review-key-recall-v0.json`)
declares `"baselines": ["null", "oracle"]`, and refusing every spec with a
non-empty `baselines` field (the literal oracle wording) would break that spec
tonight for a field this ticket does not also wire into an actual baseline
comparison. `validate` reports it as a **warning** ("declared, not yet
consumed by any runner") instead of silently ignoring it or breaking a working
spec — honest without being destructive. Wiring a real baseline comparison
(likely reusing `runs compare`'s McNemar path over a baseline config id) is
the follow-up that would let this become a hard error.

Portability (child 2) is likewise a **warning**, not a refusal: a
`daedalus_trials` corpus whose `arena_dir`/`trials_jsonl` contains `..` (both
real flagship specs do — that's how they reach the sibling `../../daedalus`
checkout) is flagged as non-portable/not-CI-runnable, informationally.
Refusing it outright would break the same two real specs backlog `016`
already tracks as a separate, not-yet-resolved hygiene item.

Child 4's "MCP tools for validate/list/get/create/update/run benchmark":
validate is now exposed over MCP (`crucible_validate`, tested with a real
stdio JSON-RPC round-trip). list/show/compare/run already existed before this
epic. "get/create/update" do not exist and are not planned as separate
tools — Crucible's specs are files on disk (git-tracked JSON), not rows in a
store; "create" is writing a JSON file (which any agent can already do
without a bespoke tool) and "update" is editing one. Inventing CRUD tools over
a file format would be exactly the "broad platform ahead of `010`" this
epic's own notes warn against.

Remaining: the baseline-comparison wiring that would let `baselines`
become a hard error instead of a warning.
