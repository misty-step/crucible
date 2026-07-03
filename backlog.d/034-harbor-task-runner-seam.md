# 034 - Harbor task runner seam

Status: open  
Priority: P1  
Owner: crucible

## Premise

Harbor is now the official harness for Terminal-Bench 2.0 and a broader
containerized agent-benchmark framework. Its task format standardizes an
instruction, `task.toml`, container environment, optional oracle solution, test
script, reward output, artifact collection, datasets, registry publishing, and
job/trial result directories.

Crucible should not replace its benchmark/run ledger with Harbor. Crucible owns
the benchmark artifact, controlled runner comparison, calibration/trust layer,
uncertainty/noise-floor reporting, and durable run records. Harbor is valuable
at the execution/task-portability seam.

## Acceptance

- Add a `harbor_task` or `harbor_dataset` runner family that can execute a local
  Harbor task/dataset through `harbor run` with a selected runner bundle.
- Parse Harbor job/trial `result.json`, verifier reward files, artifact
  manifests, and trajectories into Crucible run records without copying raw
  model outputs into tracked files.
- Preserve Crucible's comparison discipline: paired task rows where possible,
  uncertainty intervals on rates, and a plain-language noise-floor verdict.
- Support import of Terminal-Bench 2.0 tasks locally through Harbor before
  considering registry publishing.
- Document the export/publish path separately: publishing Crucible-authored
  tasks upstream is useful only after a local runner proves parity against the
  same deterministic verifier.

## Non-goals

- Do not make Harbor the source of truth for Crucible's run ledger or comparison
  semantics.
- Do not implement registry publishing before local execution and result import.
- Do not surface Harbor's viewer as the Crucible UI; link to it only as a raw
  job artifact if useful.

## Evidence

- Harbor docs: task = instruction, container environment, and test script;
  dataset = collection of tasks; job = trials over datasets/agents/models.
  <https://www.harborframework.com/docs/core-concepts>
- Harbor task format: `instruction.md`, `task.toml`, `environment/Dockerfile`,
  `solution/solve.sh`, and `tests/test.sh`; verifier writes reward files under
  `/logs/verifier/`.
  <https://www.harborframework.com/docs/tasks>
- Harbor run output: jobs directory with job config/result, per-trial
  config/result, agent trajectory, verifier logs, reward files, and collected
  artifacts.
  <https://www.harborframework.com/docs/run-jobs/run-evals>
- Harbor docs describe Terminal-Bench 2.0 running via `harbor run -d
  terminal-bench/terminal-bench-2`; Terminal-Bench now presents itself as
  Harbor-native.
  <https://www.harborframework.com/docs/tutorials/running-terminal-bench>
  <https://www.tbench.ai/>
