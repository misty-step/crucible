# 035 - Bump Sanctum instance pin for tracer-exact-v2

## Status

Open.

## Context

`tracer-exact-v2` added a new committed benchmark spec plus long-context
fixtures. The repo documents a Sanctum prepare-only posture in `README.md`, but
this checkout does not contain an obvious Sanctum/Bastion instance pin or deploy
manifest to update in-place.

Recurrence (crucible-903, investigated 2026-07-04): the crucible-902 legibility
lane merged `evals/operator-micro-benchmark-v0.json` into this repo and it hit
the identical gap — the live Bastion workbench at
`bastion.tail5f5eb4.ts.net:10000` reads specs from `/srv/crucible/evals` on the
box, and this checkout has no deploy script, systemd unit, CI workflow, or
manifest that syncs the repo's `evals/` directory to that path. `crucible
serve`'s `/api/specs` handler re-reads the mounted directory from disk on every
request (`crucible/src/serve.rs::specs_response`) — there is no in-process
cache to invalidate and no restart required — so once a spec file lands on the
box's `/srv/crucible/evals`, it is visible immediately. The entire remaining
gap for both `tracer-exact-v2` and `operator-micro-benchmark-v0` is purely
operational: someone with access to the live box (or the owning Sanctum/Bastion
deploy repo, not this checkout) needs to sync/copy the merged `evals/` spec
files onto `/srv/crucible/evals`. There is nothing to fix in the Crucible repo
itself; this item should stay open until the owning deploy manifest is found
and the sync step is made repeatable (see acceptance below, now spec-agnostic).

## Acceptance

- Identify the owning Sanctum/Bastion deployment repo or manifest for the
  Crucible `serve` instance.
- Bump the mounted Crucible revision/spec pin so `evals/tracer-exact-v2.json`,
  `evals/fixtures/tracer-exact-v2/`, and `evals/operator-micro-benchmark-v0.json`
  are available to the private instance.
- Verify `GET /api/specs` on the private instance lists `tracer-exact-v2` and
  `operator-micro-benchmark-v0`.
- Record the deployed revision and readback evidence in the lane receipt.
