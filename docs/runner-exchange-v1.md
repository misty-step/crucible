# Runner Exchange v1

Status: core envelope implemented; process transport and runner integration are
separate follow-up slices.

`crucible.runner_exchange_request.v1` and
`crucible.runner_exchange_result.v1` are the language-neutral artifact waist
described by [VISION.md](../VISION.md). They let an external runner report a
real candidate execution without adding that framework to `RunnerKind` or
teaching Crucible sibling-repository policy.

The Rust contract lives in `crucible_core::runner_exchange`. Equivalent JSON
fixtures for a Harbor-like coding agent and an unrelated raw-model adapter live
under `crucible-core/tests/fixtures/runner-exchange/`.

## Request

The request binds:

- exchange, task, adapter, candidate, harness, optional model, prompt, toolset,
  and reasoning identities;
- declared adapter capabilities;
- a content-addressed workspace/input snapshot;
- relative filesystem roots, deny-or-allowlist network authority, and
  credential **references** (`ref:<broker-path>`), never credential values;
- timeout, output, CPU, memory, and storage limits;
- repository, revision, and invocation provenance;
- evidence, transcript, usage, cost, and response-model requirements; and
- an open `adapter_payload` for runner-specific declarations.

All paths are portable and relative to the exchange root, and every input path
must fall under a declared filesystem root. The envelope grants no ambient
authority: a future transport must enforce exactly the declared limits and
must refuse when it cannot.

## Result

The result echoes the exchange, adapter, and candidate identity exactly. Its
terminal `status` is one of:

- `success`;
- `refused`;
- `timeout`;
- `malformed_output`; or
- `execution_error`.

Only `success` may claim a primary output, and every primary output must have a
relative, content-addressed evidence reference. Every non-success carries a
structured `{code, message, retryable, detail}` error instead of requiring a
caller to classify prose. Failure evidence is allowed but not confused with
the evidence required to trust a successful result.

Usage may carry input, output, and cache tokens, model cost, and latency. Cost
is optional in the base shape because a deterministic control can legitimately
have none; a request that needs cost for its decision sets `require_cost=true`,
which makes unknown cost a conformance failure while preserving explicit zero.
The same rule applies to actual response-model identity.

## Compatibility and validation

The v1 reader rejects a different or missing `schema_version`. Additive unknown
top-level fields round-trip through `extra`, and runner-owned payloads round-trip
through `adapter_payload`; neither is interpreted as Crucible policy.

Deterministic conformance validation refuses:

- blank, contradictory, or changed adapter/candidate identity;
- a direct-model candidate without model identity or a deterministic candidate
  claiming model/prompt/reasoning identity;
- absolute, parent-traversing, or non-portable input, authority, output, or
  evidence paths;
- embedded credential values in place of `ref:` names, URL-shaped network
  allowlist entries, contradictory network declarations, and zero limits;
- invalid revision/digest provenance, malformed RFC 3339 runtime timestamps,
  or a finish time before its start;
- success without output, content-addressed primary evidence, required evidence
  kinds, usage, cost, or response-model identity; and
- non-success without a structured error, or one that claims success output.

The conformance suite covers success, refusal, timeout, malformed output,
execution error, additive fields, major-version mismatch, identity
contradiction, path confinement, invalid limits/usage, and both example
adapters.

## Explicitly not implemented by this slice

This artifact contract does not spawn a subprocess, provision a sandbox,
resolve a credential, copy an artifact, or mark a run trusted. Those are the
next protocol children: a bounded stdin/stdout process driver, centralized
capability/policy projection, and a real Harbor adapter that converts its trial
receipt into this result. Until then, current `harbor_task` runs remain on their
existing execution path and cannot claim the new receipt merely because this
schema exists.
