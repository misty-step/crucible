# Live eval dashboard — evals, runs, per-config results

Priority: P1 · Status: in-progress · Estimate: L (epic)

## Goal

A live, phone-viewable dashboard that surfaces Crucible's evals, the runs driven
against them, and a defensible per-config results view — configs sorted by score,
with seed-invariant, directional, power-honest verdicts — the first end-to-end view of the eval system,
and the surface we tighten/expand/iterate on.

## Oracle

- [x] `crucible dashboard` ingests the real Threshold arenas + runs and renders a
  self-contained phone-first HTML (evals / eval-detail+results / run drill-down),
  served on the tailnet; every number reconciles with the raw trials (129/129).
- [x] The rank-gap verdict is seed-invariant, directional, and refuses below a power
  floor — "refuse a delta you cannot defend" made visible (0/117 verdicts flip across
  50 seeds; 1/117 is a defensible Signal at the current corpus).
- [ ] Drive a fresh run from the dashboard (needs live Cerberus).
- [ ] The adjudication queue (005) renders as a panel.
- [ ] Trend sparklines across arena versions.

## Delivered (2026-06-30) — v0 tracer bullet, shipped

A four-workflow fleet (deliver → thermonuclear review+QA → fix → lead-verify) shipped
v0: ingest (arena id from trials.jsonl not the dir name; config id = composition_hash;
skipped inputs counted/classified — no silent loss), the per-config results (task-clustered
bootstrap reward CI, task-level Wilson solve-rate, a 64-seed-envelope + McNemar
Bonferroni-split directional 3-state verdict, strict (arena,version) grouping), and a
self-contained HTML. The thermonuclear review caught + fixed a seed-noise verdict
(identical comparisons shipping opposite verdicts) and a directional-lie badge. Live at
`/crucible-evals` on the tailnet (python http.server + `tailscale serve --set-path`).

Honest readout: at 6-8 tasks/arena, only the oracle is a defensible Signal — the
benchmark needs more tasks/trials to rank real configs. That is the product working.

## Next (fatten the tracer bullet)

1. Drive new runs (Cerberus configs) from Crucible, not just surface Threshold's.
2. Adjudication-queue panel (005) — accept/reject disputed findings on the phone.
3. Trend view across arena versions; config diffing; cost/latency columns.
4. Power planner: "how many more tasks to make rank-gap X defensible?" (ties to 008).
5. Ingest the `cerberus-rd-lab-*` score.json format (currently classified
   unsupported); and the founding loose `*.jsonl` runs once they carry a config id.

## Notes

Consumes the measure core (003) — `bootstrap_envelope` is a new reusable primitive.
The one-scorer work (`013`) makes Crucible's own grade the scoring source
Threshold links instead of a tolerance-matched predictor; this dashboard surfaces
both Threshold-scored history and Crucible-owned runs as `010`/`011` land.

Naming: **Threshold** (formerly Daedalus) is the sibling optimization project;
the runs this dashboard surfaces are Threshold's, scored by the `daedalus-score`
binary — which keeps the `daedalus` name on disk until the sibling repo
physically renames. This is a sorted per-config results view, not a leaderboard
that crowns a winner; the verdicts stay about defensible measurement.
