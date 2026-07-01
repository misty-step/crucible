# Make specs honest and expose agent-first benchmark controls

Priority: P1 · Status: ready · Estimate: L (epic)

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

1. `crucible validate` CLI with stable JSON and human output.
2. Validation rules for portability: no hardcoded sibling paths in flagship
   specs unless explicitly marked local-only.
3. Validation rules for honest fields: graders, fixtures, baselines,
   confidence, aggregation, runner kind.
4. MCP tools for validate/list/get/create/update/run benchmark.
5. Fixture-backed authoring smoke: a cold agent creates a tiny benchmark that
   validates and runs hermetically.

## Notes

This is the agent-first surface from the operator overlay. Do not build a broad
platform ahead of `010`; make the controls thin over the real engine and typed
spec contract.
