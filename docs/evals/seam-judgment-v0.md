# Seam Judgment v0

## Decision

Use the result to decide whether a model can be trusted to propose implementation
boundaries under the fleet's model-native-first doctrine, or whether its designs
need an explicit seam-placement review before implementation.

## Capability

Given a concrete software-design seam, choose whether the seam belongs to a
model or deterministic machinery and identify the governing placement-test
rule. The benchmark does not test implementation skill.

## Corpus

The 24 tasks are balanced 12/12 between model and deterministic placement. They
are adapted from recurring fleet decisions recorded in Powder and
`roster/primitives/doctrine/model-native-first.md`: Gazette semantic
classification, Powder dispatchability, Canary incident grouping, Landmark
release synthesis, Glass report synthesis, Aesthetic visual judgment, Cerberus
review, Memory Engine extraction, Roster model routing, Crucible construct
validity, Doomscrum humor judgment, and Todoist priority judgment on the model
side; Powder leases, publication scanning, path confinement, health deadlines,
run persistence, CI gates, Mint authorization, atomic writes, schema parsing,
spend caps, backup integrity, and comparison identity on the deterministic
side.

The source ledger makes “fleet-inspired” auditable rather than decorative:

| Task | Fleet-history receipt |
|---|---|
| `gazette-event-classification` | `roster/primitives/doctrine/model-native-first.md` records the Gazette heuristic failure that established the doctrine. |
| `powder-dispatchability-triage` | Powder `roster-card-oracle-triage-v0` measures card dispatchability judgment. |
| `canary-incident-grouping` | Powder `canary-945` records a real outage whose symptoms were not joined into an alert. |
| `landmark-release-synthesis` | Landmark's declared release-intelligence surface and the fleet's shipped release-note workflow. |
| `glass-report-synthesis` | Powder `glass-941` records the reports pivot to ad-hoc synthesis. |
| `aesthetic-visual-quality` | Powder `misty-step-925` records the multi-proposal visual design pass. |
| `cerberus-semantic-review` | Crucible's existing `cerberus-review-quality-v0` eval family. |
| `memory-engine-memory-extraction` | Memory Engine's conversation-to-durable-memory product contract. |
| `roster-capability-routing` | Roster `agents/ai-scout/role.yaml` and the dated model-provider-harness capability ledger. |
| `crucible-construct-validity` | Powder `crucible-956` records construct-validity authoring lints. |
| `doomscrum-humor-verdict` | Powder `doomscrum-046` asks whether the core brainrot joke lands. |
| `todoist-priority-judgment` | The fleet Todoist skill's stale-task prioritization workflow. |
| `powder-claim-lease` | Powder's live claim/renew/expiry contract and stale-claim history on `crucible-952`. |
| `publication-secret-scan` | Powder `crucible-safe-publication-contract`. |
| `path-root-confinement` | Powder `crucible-path-confinement-http-bounds`. |
| `mint-authorization` | Powder `mint-workcell-effect-plane` and `mint-929`. |
| `provider-spend-cap` | Roster's declared capability routing includes cost constraints; the exact cap is deterministic approval policy. |
| `ci-required-gate` | Crucible `AGENTS.md` Gate and `scripts/check.sh`. |
| `canary-health-deadline` | Powder `canary-933` and the live Canary check/check-in contract. |
| `crucible-run-persistence` | Powder `crucible-011` and `crucible/src/run_store.rs`. |
| `atomic-packet-write` | Powder `crucible-safe-publication-contract` criterion 3. |
| `artifact-schema-version` | Powder `crucible-025` and the versioned artifact loaders. |
| `backup-integrity` | Powder `crucible-020` and the repository backup/restore contract. |
| `comparison-axis-identity` | Powder `crucible-974` and `crucible/src/run_store.rs`. |

The pilot models are also live fleet declarations, not arbitrary slugs:
DeepSeek v4 Flash backs Roster's `ai-scout` and `sweep` roles; GLM 5.2 is in
`roster/primitives/subagent-pool.yaml`. The benchmark does not claim they are
the only or universal fleet defaults.

Each prompt contains all facts needed for expert agreement. The two-line
expected answer is also its reference solution:

```text
PLACEMENT: MODEL|DETERMINISTIC
RULE: MEANING|TRUST|CONSUMER
```

`MEANING` means correctness requires interpreting meaning. `TRUST` means the
behavior is must-fire policy at a trust boundary. `CONSUMER` means deterministic
code consumes the output and therefore requires an exact mechanism.

## Grading and rigor

Both the placement and its justification token are graded together by an
anchored regular expression. This is intentionally narrower than grading prose:
it avoids an uncalibrated model judge while still distinguishing a lucky binary
choice from application of the correct doctrine rule.

The same tasks are run pairwise at temperature zero. Crucible supplies Wilson
intervals, paired McNemar comparison, resolution ratio, and minimum detectable
effect. `min_effect_of_interest = 0.30` acknowledges that 24 tasks can only
resolve a large routing difference; task families are correlated by source, so
the ordinary interval is optimistic and the pilot must not be read as a fleet-
wide population estimate.

## Pilot verdict rule

- Discriminating: at least one model misses four or more tasks, or the paired
  comparison clears Crucible's noise floor.
- Saturated: both models score at least 23/24; add harder mixed-rule seams before
  using it for routing.
- Broken: either model's transcript exposes a prompt ambiguity, or a reference
  answer cannot be defended directly from the placement test.

## Pilot result — 2026-07-12

The first invocation used `max_tokens = 80`; reasoning consumed the allowance
and 37/48 outputs were empty or explicitly truncated. That invocation is an
invalid run-configuration probe, not model evidence, and remains isolated in
`runs/local/seam-judgment-v0/pilot*` rather than the valid pilot ledger.

The corrected paired run used `max_tokens = 1000`:

| Model | Pass | Wilson 95% interval | Recorded response model |
|---|---:|---:|---|
| `deepseek/deepseek-v4-flash` | 23/24 (95.8%) | 79.8–99.3% | `deepseek/deepseek-v4-flash-20260423` |
| `z-ai/glm-5.2` | 24/24 (100%) | 86.2–100% | `z-ai/glm-5.2-20260616` |

The paired delta is 4.2 percentage points, `p = 1.0`, inside the noise floor.
At the observed discordance Crucible reports MDE 11.4 percentage points and
required paired `n = 181`; the pilot cannot defend a model difference. The sole
DeepSeek miss chose deterministic placement for path confinement but named the
`CONSUMER` rule rather than the intended `TRUST` rule.

**Verdict: saturated, redesign before routing use.** Both models understand the
explicit placement test on clean single-rule cases. A discriminating v1 should
remove the rule recital from the system prompt, use short architecture excerpts
where semantic and trust-boundary concerns coexist, add `DECLARATION` as a real
third answer, and source failures from proposed diffs rather than prose scenarios.
Keep v0 as a doctrine-comprehension smoke test; do not enlarge it to 181 easy
paraphrases merely to chase statistical power.
