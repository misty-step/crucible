# 035 - Bump Sanctum instance pin for tracer-exact-v2

Status: open

## Goal

`tracer-exact-v2` added a new committed benchmark spec plus long-context
fixtures. The repo documents a Sanctum prepare-only posture in `README.md`, but
this checkout does not contain an obvious Sanctum/Bastion instance pin or deploy
manifest to update in-place.

## Oracle

- [ ] Identify the owning Sanctum/Bastion deployment repo or manifest for the
  Crucible `serve` instance.
- [ ] Bump the mounted Crucible revision/spec pin so `evals/tracer-exact-v2.json`
  and `evals/fixtures/tracer-exact-v2/` are available to the private instance.
- [ ] Verify `GET /api/specs` on the private instance lists `tracer-exact-v2`.
- [ ] Record the deployed revision and readback evidence in the lane receipt.
