# Add Threshold corpus path aliases for legacy Daedalus specs

Priority: P2 · Status: pending · Estimate: S

## Goal

The committed Cerberus review-quality spec still names sibling corpus paths
under `../../daedalus/...`, while this machine's current checkout is
`../threshold`. The spec validates as a contract but cannot run from the local
checkout without a temporary path-adjusted copy under `runs/local/`.

## Oracle

- [ ] `cargo run -p crucible -- run evals/cerberus-review-quality-v0.json --out
  runs/local/cerberus-review-quality --json` works on a machine that has the
  current `threshold` checkout but no `daedalus` checkout or symlink.
- [ ] The runner reports the resolved corpus path in evidence so old and new
  names are auditable.
- [ ] Validation warnings distinguish "portable issue" from "resolvable legacy
  alias" instead of implying the run is impossible.
- [ ] No raw Threshold/Cerberus run content is committed; generated evidence
  remains under gitignored `runs/`.

## Notes

UI proof on 2026-07-02 seeded the real ledger by copying
`evals/cerberus-review-quality-v0.json` to
`runs/local/cerberus-review-quality-threshold.json` with only
`runner.corpus.arena_dir` and `runner.corpus.trials_jsonl` pointed at
`../../../threshold/...`, then running that local spec. This was acceptable
proof data, not a durable operator workflow.
