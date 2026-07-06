# Trial isolation

The contract every env-backed runner (a runner that executes candidate
behavior in a real environment rather than only grading text a caller
already produced) must satisfy, and how Crucible enforces the part of it
that is Crucible's to enforce. Written for crucible-975; referenced as a
requirement by crucible-982's end-state verification mode, and by any future
runner kind that touches a filesystem, container, or network on behalf of a
trial.

## Why this exists

Two receipts, both about the same failure mode — a trial seeing state it
should not:

- Cursor audited 731 "successful" SWE-bench-Pro-style resolutions and found
  63% were retrieval (upstream-fix lookup, git-history mining) rather than
  reasoning; strict isolation (no internet, no git history) dropped scores
  up to 20.7 points.
- Anthropic's *Demystifying Evals* names the same failure directly: shared
  state across trials both introduces correlated failures and lets a model
  cheat by reading a prior trial's artifacts.

A score inflated by leakage is not a measurement. For a workbench whose
exports feed Threshold's optimizer, leakage gets **optimized for**, not just
misreported once.

## The contract

Every env-backed runner (today: `harbor_task`; any future runner in this
family inherits this as a requirement, not a convention to rediscover):

1. **Fresh working directory per trial.** A trial's job/output directory is
   uniquely keyed by its task id and cleared of any prior contents before
   the trial starts. No trial inherits a directory another trial, or an
   earlier run of the same trial, already wrote to.
2. **No access to sibling-trial or prior-run artifacts.** One trial's job
   directory must not contain, nest inside, or otherwise expose another
   trial's job directory or output.
3. **No network beyond the declared model/agent API.** A trial calls out
   only to the endpoint its corpus config names (the OpenRouter credential
   env var, or the agent/model Harbor's config declares) — not an
   unconstrained network.
4. **Environment torn down after verification.** The sandboxed
   execution environment (today: a `harbor run` subprocess and its Docker
   container) is a fresh instance per trial and is not kept running or
   reused across trials once the trial's result is captured.

## Where each guarantee lives, and who owns it

Crucible does not reimplement a sandbox — per `AGENTS.md`'s standing
boundary ("do not reinvent eval infrastructure... Harbor... is valuable at
the execution seam"), the container/network sandbox itself is Harbor's
(and Docker's) to provide. What Crucible owns and mechanically enforces is
the **directory layer** around every `harbor run` invocation:

- `harbor_job_dir(jobs_root, task_id)` (`crucible/src/spec_run.rs`) is pure
  path construction: `jobs_root/<task_id>/<HARBOR_JOB_NAME>`. Two different
  task ids always resolve to two non-overlapping paths.
- `prepare_harbor_job_dir(jobs_root, task_id)` creates that directory and
  clears any stale contents before every run — guarantee 1 and half of
  guarantee 2 (no *own-slot* leftovers survive into the next trial that
  reuses the slot).
- `require_under_home` refuses a `task_dir` outside `$HOME` before any
  subprocess spawns, so a misconfigured task can't silently resolve to a
  path Colima won't even mount.

Guarantees 3 and 4 (network scope, container lifecycle) are Harbor's/
Docker's to provide; Crucible's contribution there is `check_harbor_available`
failing fast with an actionable message rather than a confusing mid-run
Docker error, and never holding a container open between tasks (`harbor run`
is invoked once per task, synchronously, and its subprocess exits before the
next task's invocation begins).

## The gate-level probe

`crucible/src/spec_run.rs`'s test module carries the leakage probe this
contract requires, run by `cargo test --all` (part of `scripts/check.sh`'s
slow gate) on every push — no live `harbor`/Docker install needed, because
these tests exercise the real `prepare_harbor_job_dir`/`harbor_job_dir`
production code paths directly rather than shelling out to the `harbor` CLI
(the same pattern this file's `read_harbor_trial_result_*` tests already
use, since CI does not install `harbor`):

- `harbor_job_directory_clears_prior_trial_artifacts_before_reuse` — plants
  a marker file simulating a prior trial's leftover artifact, prepares the
  same task id's job directory again, and fails if that marker is still
  visible. This is the literal "attempt to read a prior trial's state and
  come back empty" probe guarantee 1 requires.
- `harbor_job_directories_are_disjoint_across_task_ids` — plants a marker in
  one task id's job directory, prepares a *different* task id's job
  directory, and fails if the second directory nests inside, contains, or
  otherwise exposes the first.

Both tests pass against the runner as it exists today — `HarborTask`
already satisfies this contract; the tests exist so a future change cannot
silently regress it without the gate catching it.

## What this contract does not (yet) cover

- It does not probe Harbor's own container network policy or filesystem
  sandbox — that would require a live `harbor`/Docker-based integration
  test, which CI cannot currently run (no `harbor` install step). A
  container-level probe is future work, not claimed as covered here.
- `agentic_judge` and `prompt_benchmark` make no filesystem/container
  changes on a trial's behalf today (each task is one stateless model
  call), so they have no working-directory surface for this contract to
  apply to. If either grows one, it inherits this contract exactly as
  `crucible-982`'s end-state verification mode is required to.
