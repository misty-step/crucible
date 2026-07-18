# Harbor Consumer Contract

Status: characterized against Crucible `master` and Harbor 0.13.1 on
2026-07-13. This is the generic engine contract for an external repository
such as Bench. It is not a benchmark definition.

## Package and import

A consumer owns ordinary Harbor task directories. The current generated task
shape contains:

- `task.toml` with a top-level `version` and `[verifier]`, `[agent]`, and
  `[environment]` sections;
- `instruction.md`;
- `environment/Dockerfile`;
- a verifier such as `tests/test.sh`; and
- optionally `solution/solve.sh` for Harbor's zero-model `oracle` agent.

Crucible projects a directory of those tasks into an `EvalSpec` through its
public CLI:

```sh
crucible import harbor benchmarks/example/tasks \
  --id example-v0 \
  --task-family example \
  --decision "The decision this benchmark changes" \
  --agent oracle \
  --out benchmarks/example-v0.json \
  --json
```

Every directory entry is imported or named as skipped. The importer performs a
light structural recognition check, not a shadow Harbor schema validation;
`harbor run` remains authoritative for the full package. It accepts current
generated tasks and legacy local tasks with a `[task]` section. Imported task
paths are rebased relative to the output spec, so moving the checkout preserves
their meaning.

## Validation and execution

`crucible validate` requires `runner.kind=harbor_task`,
`corpus.source=harbor_tasks`, at least one task, a deterministic grader,
proportion aggregation, and Wilson 95% uncertainty. Validation does not build
the container or run the verifier.

At execution, Crucible requires `harbor --version`, a working `docker info`,
`$HOME`, and both task and run-output paths under `$HOME`. Colima exposes that
tree to its Docker VM by default; Crucible refuses either path outside it before
spawning Harbor, rather than persisting a misleading ordinary 0/N run when
Harbor cannot collect `reward.txt`. It invokes one synchronous process per task:

```text
harbor run -p <task_dir> -a <agent> -o <task_jobs_dir> --job-name run -y
```

A custom agent instead uses exactly one import-selection flag:

```text
harbor run -p <task_dir> --agent-import-path <module:Class> -o <task_jobs_dir> --job-name run -y
```

When `agent_import_path` is set, Crucible resolves the EvalSpec's parent directory
as the custom module import root and supplies it to the Harbor host process through a
child-process-only `PYTHONPATH` (prepended to any existing `PYTHONPATH`). The
required `agent` value remains the stable receipt/config identity label. Blank import
paths are rejected before Harbor starts. When declared, `model` adds `-m <model>`. Crucible applies a wall-clock timeout
to the whole subprocess. Each task gets a cleared, disjoint job directory;
Harbor and Docker own container teardown and the in-container sandbox. The
mechanical directory guarantees and their limits are in [docs/AGENTS.md](AGENTS.md).

The `HarborRunConfig` currently applies `agent`, optional `agent_import_path`, `model`, and
`job_timeout_ms`. A declared `resource_envelope` is persisted for comparison
caveats but is not translated into Harbor CPU or memory flags. Reasoning
effort, agent kwargs, tools, skills, MCPs, memory, network allowlists, and role
topology cannot yet be applied or identity-hashed by this runner.

## Evidence, storage, and comparison

The runner writes `crucible.harbor_run_evidence.v1` with:

- agent identity, optional custom `agent_import_path`, optional requested model, and
  optional declared resource envelope;
- Wilson score and task totals;
- task id and resolved task path;
- full reward and reward breakdown;
- elapsed task time, verifier summary, exception state, and Harbor result JSON;
- verifier reward/test output and Harbor artifact-manifest pointers when they
  exist.

The SQLite ledger indexes every Harbor task outcome. `runs show` returns the
indexed row plus the underlying evidence. Config identity is currently
`harbor:<agent>:<model-or-default>` and exposes the Harbor agent as `harness`.
Two runs over shared task ids receive a paired McNemar comparison; no shared
tasks falls back to an unpaired descriptive delta. A changed agent alone is a
`harness_delta`. Resource-envelope mismatch or absence is caveated, not hidden.

All non-judge Harbor runs are currently persisted as `trusted=true`. That means
trust only says the existing deterministic Harbor reward path was measured; it
does not license an architectural model judge, held-out-generalization claim,
clustered population claim, or public disclosure.

## Operator surfaces

`crucible serve --specs <consumer-spec-dir> --db <ledger>` lists and validates
Harbor specs, renders their task ids, and exposes stored Harbor runs and
per-task evidence through the run detail surface. Harbor specs report
`supports_controlled_comparison=false`: the current browser setup/launch flow
is limited to deterministic `prompt_benchmark` specs. CLI execution and ledger
comparison are the working control surface.

A Harbor run does not produce an adjudication queue. The existing label panel
therefore has nothing to open for a normal Harbor result. Architectural or
human feedback needs a generic task-level judgment artifact and calibrated
label path before it can affect a trusted Seam Agency verdict.

`crucible publish` refuses `harbor_task`. Packet v1 only accepts
`prompt_benchmark`, and raw agent diffs/transcripts must not be forced through
that path.

## Live characterization receipt

The committed `current-template-smoke` fixture is a current-format, clean-room
Harbor task. On 2026-07-13 it was imported, validated, and run locally through
Harbor 0.13.1 and Docker 29.2.1 using already-present container layers:

- `oracle` applied the reference solution and passed 1/1; the run report's UTC
  completion timestamp was `2026-07-13T20:26:11.443Z`;
- `nop` made no change and failed 0/1;
- `runs show` exposed reward, verifier output, duration, artifacts, and the
  Harbor result;
- strict comparison paired the shared task and correctly reported the 1-task
  difference as inside the noise floor and underpowered; and
- publication refused because `harbor_task` is unsupported.

The ignored local receipt lives under
`runs/local/harbor-current-template-smoke*`; it is diagnostic evidence, not a
publishable benchmark result.

## Seam Agency handoff

Bench can now target the following unprivileged first proof:

1. own standard Harbor task directories and a Bench methodology package;
2. run `crucible import harbor` into a Bench-owned `EvalSpec`;
3. validate and execute references with `agent=oracle`;
4. prove named no-op or wrong-seam implementations fail through task verifiers;
5. inspect the resulting runs through CLI and `crucible serve`.

Do not call a broad model/harness matrix trusted or publishable yet. The next
consumer proof must first resolve or explicitly refuse composition identity,
real external-runner receipts, human/model architectural labels, held-out and
clustered inference, browser launch parity, and safe agentic publication.

Existing Powder ownership:

- `crucible-runner-artifact-protocol`: versioned external composition and
  evidence envelope;
- `crucible-run-space-workbench`: model/harness/articulation launch UI;
- `crucible-955` and `crucible-964`: held-out governance and clustered
  uncertainty;
- `crucible-safe-publication-contract` and `bench-packet-acceptance-gate`:
  fail-closed publication; and
- `crucible-seam-agency-consumer-proof`: the end-to-end Bench consumer proof.

The first child of `crucible-runner-artifact-protocol` is now the core-only
[Runner Exchange v1](runner-exchange-v1.md) request/result contract. It defines
the identity, authority, limits, evidence, usage/cost, trust, and structured
error waist, but does not yet adapt or replace the existing Harbor execution
path. A Harbor coding-agent result becomes evidence under that contract only
after the bounded process driver and real adapter land.
