# Threshold eval-authoring map (epic 007.1)

Read-only survey of the eval/benchmark-**authoring** machinery that lives in
Threshold today, classified `MIGRATE → Crucible` vs `STAY` (Threshold optimization
loop) vs `SHARED` (the contract surface between the two repos). This is the
input to epic `007` (extract eval-authoring from Threshold); it is a map, not a
change. No Threshold file was modified.

> Naming: the sibling repo is **Threshold** (formerly Daedalus). It has not
> physically renamed yet, so its on-disk checkout (`…/daedalus`), crates
> (`daedalus-core`, `daedalus-cli`), and the `daedalus-score` binary keep the
> `daedalus` name; every path in this map is real and unchanged.

- **Surveyed:** `/Users/phaedrus/Development/daedalus` @ `b48c608`
  (_docs: clarify Daedalus frontier output_ — commit subject, verbatim), 2026-06-30.
- **Boundary being realized** (per `AGENTS.md` / `VISION.md`): Crucible owns the
  eval/benchmark as a durable artifact — definition, design, scoring design,
  calibration, run records, judging, reporting, export. Threshold runs
  Karpathy-style config-optimization loops that **consume** Crucible's trusted
  evals through the Harbor contract. "Ultimately extract most of that from
  Threshold" (operator, /groom 2026-06-29). Authoring-rights are ratified; the
  cross-repo ownership handshake (007 child 2) is still pending — this map exists
  to make that handshake concrete.

> Governance note: do **not** unilaterally edit Threshold. Migration order is
> child 2 (agree ownership) → child 3 (migrate code-review family) → child 4
> (corpus/holdout/contamination governance) → child 5 (narrow Threshold, delete
> migrated machinery). This document only feeds child 1→2.

---

## At a glance

| # | Surface | Where (Threshold) | What it is | Verdict |
|---|---|---|---|---|
| 1 | Arena definitions + task dirs | `arenas/<id>/` (`arena.toml`, `template.md`, `tasks/<id>/{task.toml,intent.md,environment/,tests/,solution/}`) | The eval corpus: fixtures, seeded defects, splits, instructions | **MIGRATE** |
| 2 | Answer keys (2 shapes) | `tasks/<id>/solution/findings.json` (oracle) + `tasks/<id>/tests/expected.json` (scorer key, line-spans) | Ground truth a review is scored against | **MIGRATE** |
| 3 | Task specifications | `specs/<family>/taskspec.toml` | Declarative eval definition (goal, inputs, output contract, oracle, budget, gates) | **MIGRATE** |
| 4 | Scoring design + scorer | `crates/daedalus-core/src/score.rs`; `crates/daedalus-score` (musl binary) | `reward = max(0, recall − 0.2·FP)`, clean-task hard-zero, matcher, key red-team audit | **MIGRATE** design · **SHARED** binary |
| 5 | Adjudication / key-extension | `arenas/<id>/adjudications.md`; CLI `ArenaAdjudicate`, `ArenaDisagreements` | ACCEPT / OUT-OF-SCOPE log that *extends the answer key* + bumps arena version | **MIGRATE** |
| 6 | Holdout exposure ledger | `arenas/<id>/holdout-ledger.md` | Burn-after-5-exposures discipline; appended at `--final` scoring | **MIGRATE** data · **SHARED** write-protocol |
| 7 | Contamination ledger | `arenas/<id>/contamination.toml` | Machine-readable train/eval contamination record (per-source novelty) | **MIGRATE** |
| 8 | Arena-authoring CLI | `crates/daedalus-cli` verbs: `ArenaScaffold/Validate/Freeze/Redteam/Adjudicate/Disagreements`, `TaxonomyValidate`, `Score` | The author→calibrate→freeze→adjudicate toolchain | **MIGRATE** |
| 9 | Taxonomy | `crates/daedalus-core/src/taxonomy.rs`; `TaxonomyValidate` | Finding-category vocabulary the matcher keys on | **MIGRATE** |
| 10 | Harbor build / port | `crates/daedalus-core/src/port_harbor.rs`; `harbor-build/<id>/`; `bin/harbor-run` | Packages an authored arena into the Harbor task-directory format the loop runs | **SHARED** (format) · port = MIGRATE, runner = STAY |
| 11 | Cerberus handoff packets | `cerberus.rs`, `prompt_packet.rs`; `ExportCerberus`, `ExportSuite` | ReviewerConfigPacket / suite-contract export | **SHARED** (contract) |
| 12 | Optimization loop | `run.rs`, `search_loop.rs`, `mutate.rs`, `seed.rs`, `swarm.rs`, `lineage.rs`; CLI `Run/Compare/Basin/View/ReportHtml/Trace/Regression/Doctor`, `Export/LaunchPack`, `CerberusLab` | Search compositions, mutate configs, certify, report, deliver | **STAY** |
| 13 | Delta statistics | `crates/daedalus-core/src/stats.rs` (cluster-robust SE, reward-delta CIs) | Confidence-bounded run-vs-run deltas | **SHARED / converge** |
| 14 | Validation kernel | `crates/daedalus-core/src/validate.rs` | Schema/receipt/contract validation | **SHARED / split** |

---

## 1–2 · Arenas, task dirs, answer keys — MIGRATE

Each arena is a directory under `arenas/<id>/` in the **Harbor task-directory
format** (`arena.toml` `description`):

```
arenas/pr-review-v0/
  arena.toml              # id, version, taskspec ref, [split] train/validation/holdout, [risk], frozen-surface contract
  template.md             # shared instruction text, composed with each task's intent.md
  contamination.toml      # surface 7
  holdout-ledger.md       # surface 6
  adjudications.md        # surface 5
  provenance.md           # arena authorship/provenance (some arenas)
  tasks/<task-id>/
    task.toml             # id, source_repo, [agent]/[verifier] timeouts
    intent.md             # the PR's stated intent
    environment/          # post-change files + PR.diff (the config's workspace)
    tests/                # verifier (test.sh) + hidden scorer key (expected.json)
    solution/             # oracle findings (findings.json)
```

Live arenas: `pr-review-v0` (contamination-resistant synthetic holdout),
`pr-review-v1`/`-v2` (real-repo snapshots), `pr-review-master-v0`,
`pr-review-security-v0`, `pr-review-correctness-v0`, `launch-contract-v0`,
`cerberus-fixture-v0`. `arena.toml` freezes the eval: fixtures, answer keys,
scorer constants, template, and the `[split]` train/validation/holdout — any
change requires a version bump and a baseline re-run (`pr-review-v0` is at
`0.3.0`; its changelog records that key extensions invalidate cross-version
averaging). This versioned, frozen, calibrated artifact **is** the thing the
recharter says Crucible owns.

**Two answer-key shapes per task** (both MIGRATE; Crucible already consumes the
first):

- `solution/findings.json` — `{ "findings": [{ file, line, category, severity?,
  description }] }`, the human-readable oracle. `severity` is omitted by roughly
  half of real keys (present on 16 of 39 live findings) and absent on the rest.
  This is the exact shape `crucible-core/src/key.rs::KeyFinding` parses (with
  `severity` as `#[serde(default)]` precisely because of that split) — Crucible
  is **already** the downstream reader of this format.
- `tests/expected.json` — `{ "defects": [{ id, file, line_start, line_end,
  category, severity?, note }] }`, the machine scorer key with **line spans** and
  per-defect ids. This is the file `daedalus-score` reads and scores against
  (confirmed across all 39 live arena defects: each carries an `id`, a
  `[line_start, line_end]` span, `category`, and a free-text `note`; `severity`
  is present on a minority, 16/39). A finding scores a hit on `file == file &&
  category == category && line ∈ [line_start, line_end]`; `note` is the human
  rationale the scorer ignores — the span-key analogue of the oracle's
  `description`, and the field `crucible-core/src/key.rs::Defect` already models.

Reconciling these two representations (span-key vs point-oracle) under one
Crucible-owned schema is a concrete child-3 task. Configs under test must never read
`tests/` or `solution/`; only `environment/` is copied into the agent workspace.

## 3 · Task specifications — MIGRATE

`specs/<family>/taskspec.toml` is the declarative eval definition referenced by
`arena.toml` (`taskspec = "specs/pr-review/taskspec.toml"`). It carries `goal`,
`domain`, `mode`, `[inputs]` (which arena is canonical), `[output]` contract
(`findings.json: {findings:[{file,line,category,description}]}` over a fixed
taxonomy), `[oracle]` (`type = "deterministic"`, the reward formula),
`[budget]` (per-trial cost/wall ceilings), and `[checkpoints]` gates
(`G1-spec`, `G2-eval-quality`, `G3-launch-contract`). Of these, the eval
definition (goal/inputs/output/oracle/taxonomy) is Crucible-owned; the budget +
trigger fields lean toward the loop and are a clean split point. The G2
eval-quality checkpoint is the calibration/trust gate Crucible owns; G1/G3 are
shared governance.

## 4 · Scoring design + scorer — MIGRATE (design) / SHARED (binary)

`crates/daedalus-core/src/score.rs` is the deterministic grader (a faithful port
of the retired `runner/score.py`):

- **Reward:** `reward = max(0, recall − 0.2·false_positives)`, **except** a clean
  task (empty key) scores a hard `0` if it surfaces any finding — inventing
  defects on a sound change fails the task's whole point.
- **Match:** a finding matches a defect on `file == file && category ==
  category && line ∈ [line_start, line_end]` (+ optional severity-at-least-as-
  strict via a `blocking>serious>minor` rank); greedy, each defect at most once;
  unmatched findings are false positives; a missing/malformed `findings.json`
  scores `0`.
- **`redteam_audit`** (backlog 040): flags answer keys whose line-spans are wide
  enough that a config could score by guessing `file+category` without
  localizing — a key-quality (calibration) tool.

This **scoring design** is squarely what the recharter assigns to Crucible
("definition, design, … calibration"). Crucible's own
`crucible-core/src/grade.rs` already re-implements the matcher half
(`key_match`, `dedup`, category-strict matching, `recoverable_misses`) on the
`findings.json` shape — so the design has *already started* migrating; `score.rs`
is the reward/penalty + span-matching layer not yet mirrored.

`crates/daedalus-score` is a tiny `main.rs` wrapping `daedalus_core::score::score`
into a static **musl** binary (`daedalus-score <findings.json> <expected.json>`)
that the Harbor container invokes. The binary is the **SHARED** boundary: Crucible
should own/produce the scorer; the Harbor runner executes it. `bin/harbor-run`
builds it (`cargo build --release --target x86_64-unknown-linux-musl -p
daedalus-score`) before each run.

## 5 · Adjudication / answer-key extension — MIGRATE (already mirrored)

`arenas/<id>/adjudications.md` is an append-only ACCEPT / OUT-OF-SCOPE log: when
a human (or the queue) rules that a config's finding the key missed is a real
defect, it is ACCEPTed, **added to the answer key, and the arena version is
bumped** (e.g. `pr-review-v0` `0.3.0`: "py-file-cache key extended with
tmp-write-race (concurrency) via adjudication ADJ-1"). CLI `ArenaAdjudicate`
appends entries; `ArenaDisagreements` reports the category/span misses that feed
them.

This is the single strongest MIGRATE signal: Crucible's `export.rs`
(`render_adjudications_md` / `parse_adjudications_md`, the
`crucible.judgment_queue.v1` → `adjudications.md` round-trip) is **explicitly
built to emit this exact artifact**. The adjudication → key-extension loop is the
heart of the code-review wedge (002) and the calibration/trust layer — it belongs
in Crucible, with Threshold reading the resulting versioned key via Harbor.

## 6–7 · Holdout & contamination governance — MIGRATE (corpus governance, child 4)

- `holdout-ledger.md` — every `--final` scoring of a holdout task is appended
  here (the run does it automatically at "stage 4"); a holdout task burns after
  **5 exposure entries** and must be rotated into train/validation and replaced
  (version bump). The *ledger and the burn discipline* are corpus governance
  (MIGRATE); the *append at scoring time* is a write the optimization run
  performs (**SHARED** write-protocol — Threshold must report exposures back to
  the Crucible-owned ledger after migration).
- `contamination.toml` — machine-readable record of whether an arena's defects
  are publicly indexable (train/eval contamination), with per-`source`
  `public`/`repo` fields and `defects_novel`. It is what lets a result claim
  "this score is not inflated by training-data familiarity." Pure corpus
  governance → MIGRATE (007 child 4 names exactly this).

These two ledgers are the trust/provenance layer around the corpus and map
directly onto Crucible's "refuse to report a delta it cannot defend" principle.

## 8–9 · Arena-authoring CLI + taxonomy — MIGRATE

The `crates/daedalus-cli` (`daedalus`) verb surface splits cleanly into an
**authoring** half and an **optimization** half. The authoring half is the
toolchain Crucible should own:

| CLI verb | Role |
|---|---|
| `ArenaScaffold` | create Harbor-format task placeholders |
| `ArenaValidate` | validate an arena freeze gate (no model spend) |
| `ArenaFreeze` | freeze packet: oracle, null, one-shot probe, report |
| `ArenaRedteam` | flag gameable wide-span keys (calls `score::redteam_audit`) |
| `ArenaAdjudicate` | append ACCEPT / OUT-OF-SCOPE (surface 5) |
| `ArenaDisagreements` | report category/span misses vs a key |
| `TaxonomyValidate` | validate a review-swarm taxonomy against a suite taskspec |
| `Score` | score `findings.json` vs a key (surface 4) |

`crates/daedalus-core/src/taxonomy.rs` defines/validates the finding-category
vocabulary the matcher is strict on; since both the key and the matcher key on
`category`, the taxonomy is part of the eval definition → MIGRATE.

## 10–11 · Harbor build + Cerberus handoff — SHARED contract surfaces

The **Harbor task-directory format is the consumption contract** and is where
the boundary actually lives:

- `port_harbor.rs` + CLI `PortHarbor` render an authored arena into
  `harbor-build/<id>/<task>/` (`instruction.md` composed from template+intent,
  `task.toml`, `environment/`, `tests/`, `solution/`). The *packaging* is
  authoring-side (MIGRATE — Crucible exports to Harbor), the *format* is SHARED,
  and `bin/harbor-run` (build musl scorer → port → `harbor run --agent
  pi|oracle`) is loop execution (STAY). Crucible's export contract already
  targets "the Threshold Harbor task-directory format" per `AGENTS.md`.
- `cerberus.rs` / `prompt_packet.rs` + `ExportCerberus` / `ExportSuite` emit the
  `ReviewerConfigPacket.v1` / suite contracts that hand a tuned reviewer config
  between the repos — a SHARED interface, not eval-authoring logic.

## 12–14 · STAY (the optimization loop) + the gray zone

**STAY** — the Karpathy optimization loop and its outputs, which *consume* evals
but do not define them: `run.rs` (search), `search_loop.rs` (ports
`runner/loop.py`), `mutate.rs` (candidate-config moves), `seed.rs`, `swarm.rs`
(review-swarm execution), `lineage.rs`, and the CLI verbs `Run`, `Compare`,
`Basin` (seed-disagreement detector), `View`/`ReportHtml`/`Trace` (run
reporting), `Regression`, `Doctor`, `Export`/`LaunchPack` (control-plane
delivery), `CerberusLab` (import review artifacts into lab evidence). The
Python-compat shims `pycompat.rs` / `pyrandom.rs` are port scaffolding (STAY,
delete-eligible post-migration).

**Gray zone to resolve in the ownership handshake:**

- `stats.rs` (**SHARED / converge**) — cluster-robust SE and reward-delta CIs.
  Crucible's `measure` module (Wilson intervals, Cohen's κ, paired
  `DeltaVerdict`, bootstrap, power) is the same measurement-rigor surface for the
  *eval*; Threshold needs delta stats for *loop certification*. Decide whether one
  Crucible-owned rigor crate serves both, or each keeps a copy at its own
  altitude.
- `validate.rs` (**SHARED / split**) — schema/receipt/contract validation spans
  eval-artifact schemas (Crucible) and run-record schemas (Threshold). Split along
  the artifact owner.
- `taskspec.toml` budget/trigger fields and the `approvals/` G1/G3 gates lean to
  the loop; G2 (eval-quality) is Crucible's calibration gate.

---

## Migration summary

- **MIGRATE → Crucible:** arenas + task dirs + both answer-key shapes (1–2),
  task specs (3), scoring design + matcher + key red-team (4, design half),
  adjudication/key-extension (5), holdout + contamination governance (6–7),
  the arena-authoring CLI (8) and taxonomy (9), Harbor *packaging* (10, port
  half).
- **SHARED contract (name owner, keep one definition):** Harbor task-directory
  format + the musl scorer binary (4/10), Cerberus handoff packets (11), the
  holdout-ledger write-protocol (6), `stats`/`validate` at the eval/loop seam
  (13–14).
- **STAY (Threshold optimization loop):** search/mutate/seed/swarm/lineage and the
  run/compare/report/deliver CLI surface (12), plus the Harbor *runner* and
  port-compat shims.

**Already in flight** (de-risks child 3): Crucible's `grade.rs` mirrors the
matcher, `export.rs` mirrors `adjudications.md` round-tripping, `key.rs` already
parses `solution/findings.json`, and `measure` covers the eval-side of `stats`.
The code-review family is therefore the right first migration: its authoring
surface is partly re-homed already, and the only hard contract to hold steady is
Harbor + the scorer binary.

**Open questions for the ownership handshake (007 child 2):**

1. Who holds the canonical `arenas/` corpus after migration — Crucible repo,
   with Threshold consuming via a pinned Harbor export? (Recommended.)
2. One shared scorer crate published to Harbor, or Crucible owns
   `score`/`expected.json` design and Threshold keeps the musl build target?
3. Reconcile the two answer-key shapes (`solution/findings.json` point-oracle vs
   `tests/expected.json` span-key) into one Crucible schema, or keep both with a
   generator?
4. Holdout/contamination ledgers: Crucible-owned files that the Threshold run
   appends exposure entries to — agree the write-back protocol so the burn
   discipline survives the split.
