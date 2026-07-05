# Crucible Design References

The external canon Crucible's design answers to. Sourced from the operator's
study syllabus ("The Study Syllabus — Evals, Agents, and the Supporting
Literacies," Sanctum shelf `artifacts/a/study/`, ~95 live-verified resources,
2026-07-04) plus a fresh research sweep on 2026-07-05. Seven reading lanes
distilled every resource against Crucible's live state; the full lane reports
are archived in the daybook vault
(`meta/briefs/2026-07-05-crucible-design-reference-lanes/`).

How to read this: each section names the references that govern one Crucible
subsystem, the design rules they impose, and the concrete deltas they imply.
"Already holds" marks where Crucible's existing design independently converged
with the canon — those are load-bearing validations; keep them true. Deltas
are tracked as Powder cards (repo: crucible); this document is the durable
*why* behind them.

The one-line summary of the whole sweep: **the literature does not overturn
Crucible's design — it confirms the spine (intervals, paired stats, noise
floor, calibrated judges, gaming canaries, deterministic-first grading) and
then names the next layer of rigor at every seam.**

---

## 1. The verdict layer (statistics)

References:

- Miller, *Adding Error Bars to Evals* (arXiv:2411.00640) — evals as
  experiments over a question super-population; clustered SEs when items are
  grouped; paired-difference inference on shared question sets; power
  analysis / minimum detectable effect (MDE); never lower temperature as a
  variance hack.
- Bowyer, Aitchison & Ivanova, *Don't Use the CLT Under a Few Hundred
  Datapoints* (arXiv:2503.01747) — CLT-family intervals dramatically
  underestimate uncertainty at small N; bootstrap also underperforms there;
  recommends Wilson/Agresti-Coull-family intervals and Bayesian credible
  intervals (`bayes_evals` library).
- Kotawala, *Resolution Diagnostics for Paired LLM Evaluation*
  (arXiv:2605.30315) — resolution ratio q = N/N\*; 11/40 Open LLM Leaderboard
  and 4–6/9 MMLU-Pro adjacent-rank pairs are unresolved at (α=.05, power=.8);
  the common unpaired-Cohen-h-times-(1−ρ) power shortcut is wrong by ~2×
  exactly in the close-call regime (`llm-power` tool).
- Evan Miller, *How Not To Run an A/B Test* — peeking inflates a nominal 5%
  false-positive rate to ~26%; fix N in advance or use sequential methods.
- Johari, Pekelis & Walsh, *Always-Valid Inference* (arXiv:1512.04922) —
  confidence sequences that stay valid under continuous monitoring.
- Reinhart, *Statistics Done Wrong* (ch. 3, 5, 7) — underpowered studies;
  truth inflation: effects selected for clearing a threshold are
  systematically overestimated.
- Holm-Bonferroni (preferred over plain Bonferroni) for family-wise error on
  comparison grids; Arawjo, *statsforevals.com* / `evalstats` — automatic
  method selection by data type × N, simultaneous CIs with built-in
  multiple-comparison correction across model×prompt grids, 99% default CI as
  deliberate strictness.
- Kohavi, Tang & Xu, *Trustworthy Online Controlled Experiments* — Twyman's
  Law ("any figure that looks interesting is usually wrong"), sample-ratio
  mismatch as the first check on a surprising result, OEC discipline.
- Wilson-interval literacy (statisticsfundamentals.com; Wikipedia binomial CI
  comparison) — Wilson right over Wald; Jeffreys as the equal-tailed Bayesian
  sibling with Wilson-grade coverage.
- Bayesian Dirichlet framework for pass@k (arXiv:2510.04265) — closed-form
  credible intervals that extend to graded/rubric scores, where Wilson
  (binary-only) does not apply.
- Masood's statistical-rigor menu — McNemar for paired binary, Wilcoxon
  signed-rank for paired ordinal/non-normal, Krippendorff's α for multi-rater
  agreement with missingness, Bradley-Terry over raw Elo for pairwise
  aggregation (the migration Chatbot Arena itself made).
- Yan, *Task-Specific LLM Evals* — separation-of-distributions diagnostic
  (e.g. Jensen-Shannon divergence between per-class judge-confidence
  distributions): "is this threshold even stable?" is a different question
  from "is this rate real?"

Already holds: Wilson over Wald; McNemar paired comparison on shared task ids
(`PairedComparison::mcnemar`); `DeltaVerdict::{Signal, InsideNoiseFloor}` as
persisted verdicts; pass^k refusing to compute over unequal trial counts.

Deltas:

- **Resolution/power diagnostic.** Report q = N/N\* and the MDE with every
  comparison, and warn at authoring/validate time when the declared N cannot
  resolve the decision the eval claims to inform. Turns the noise floor from
  a retrospective gate into a prospective design constraint.
- **Cluster keys on fixtures.** Cerberus findings cluster by diff/PR; naive
  i.i.d. intervals are silently too narrow. Declare a cluster key in the
  EvalSpec; switch to clustered SEs when set.
- **Multiplicity correction on grids.** `runs pivot` and any dashboard grid
  showing many pairwise verdicts needs Holm-Bonferroni (or an explicit
  "N comparisons, uncorrected" label). Findings selected as "biggest delta"
  from a grid carry a truth-inflation caveat in the findings journal.
- **Small-N regime handling.** Below a few hundred datapoints, flag the
  regime and add a Jeffreys/Bayesian credible interval alongside Wilson; for
  graded (non-binary) scores use a Dirichlet-posterior interval; never fall
  back to bootstrap.
- **Right paired test per data type.** McNemar is correct for paired binary;
  paired ordinal/graded scores need Wilcoxon signed-rank; pairwise-preference
  aggregation should be Bradley-Terry. The comparison layer should select by
  data type, not apply one generic test.
- **Peeking policy.** Decide and enforce one of: (a) N locked at run start,
  or (b) anytime-valid confidence sequences if a live "watch this run"
  surface lets anyone stop early. The serve dashboard makes this a real, not
  theoretical, hazard.
- **Twyman/SRM guard.** A surprisingly large delta triggers an automatic
  sanity check (fixture-set identity, run completeness) before it can become
  a reactable finding.
- **Threshold-stability diagnostic.** Where a grader emits a continuous
  confidence, surface per-class distribution separation before trusting the
  binarized rate.

## 2. Benchmark design & validity (the eval object)

References:

- *Measuring What Matters: Construct Validity in LLM Benchmarks*
  (arXiv:2511.04703; 29 experts, 445 benchmarks audited) — only 53% justify
  construct validity, only 16% use any statistics; eight recommendations:
  define the phenomenon, control confounds, representative sampling,
  dataset-reuse limits, contamination prep, statistics, error analysis,
  explicit validity justification.
- Anthropic, *Demystifying Evals for AI Agents* — the 8-step roadmap: start
  with 20–50 tasks from real failures; reference solutions must themselves
  pass all graders; balanced problem sets (test when behavior should *and*
  shouldn't occur); 0% pass@100 usually means a broken task, not an incapable
  model; capability evals (low pass rate, a hill to climb) are structurally
  distinct from regression evals (near-100%, decline = alarm) and graduate
  into them; monitor for saturation.
- *Life After Benchmark Saturation: CORE-Bench* (arXiv:2606.26158) —
  saturation is diagnostic, not a retirement trigger: models tied on accuracy
  diverged sharply on cost (60% cheaper) and calibration (32% self-reported
  confidence vs 93% actual).
- SWE-bench Verified → SWE-Bench Pro — human filtering, then a public /
  held-out / commercial split specifically to expose overfitting via the
  public-vs-held-out gap; harness alone swung SWE-bench-Lite 2.7%→28.3% for
  the same model.
- *Dissecting the SWE-Bench Leaderboards* (arXiv:2506.17208) — harness
  engineering vs model capability attribution.
- ARC-AGI-2 / ARC Prize 2025 report — human-calibration floor
  (easy-for-humans, hard-for-AI); spend-once semantics on held-out scores; a
  separate leaderboard category isolating scaffold gains from model gains;
  "knowledge overfitting" without direct memorization.
- Humanity's Last Exam — private held-out set alongside public; univocal
  answers even at expert difficulty.
- Contamination literacy: *What Is a Contaminated LLM?* (ConStat definition;
  paraphrase-gap detection); Infini-gram mini (arXiv:2506.12229) — cheap
  exact lexical-overlap screening at scale; LiveBench — monthly-refreshed,
  objective-ground-truth-only design; *Contamination-Resistant Datasets*
  (arXiv:2605.19999); FACTS v2 (DeepMind) and GDPval (OpenAI) — public gold
  subset + private held-out split + neutral hosted scorer as the 2026
  distribution pattern.
- *Questionable Practices in ML* (arXiv:2407.12220) — 44 QRPs; harness
  hacking (same model, ~30-point MMLU swing from prompt formatting alone);
  subset/runtime/prompt/metric hacking; API drift.
- *When Benchmarks Are Targets* (arXiv:2402.01781) — MCQ perturbations shift
  rankings up to 8 positions; hybrid scoring over symbol-only.
- Karpathy, *2025 Year in Review* — "benchmaxxing"; verifiable-reward
  benchmarks are RLVR-gameable by construction.
- Saturation at the field level: MMLU-family functionally saturated above
  ~88% — top-model differences on saturated instruments are statistically
  meaningless.
- HELM — multi-metric vector, never one collapsed score.
- Husain, *"It's Hard to Eval" Is a Product Smell* — if a task's output has
  no checkable sub-units, the eval will be noisy no matter how good the
  judge; authoring should ask "what would a human reviewer check
  line-by-line?" first.

Already holds: EvalSpec carries the decision-the-eval-informs, baselines, and
a declared grader mix; config identity carries model/harness/tool_allowlist;
`validate` refuses unsupported aggregation/uncertainty; specs are honest about
what they measure.

Deltas:

- **Construct-validity section in `crucible validate`.** Phenomenon defined
  and uncontested? Sampling strategy declared? Error analysis planned?
  Validity justification written? The 2511.04703 checklist is concrete
  enough to be a literal gate section (warnings first, not hard failures).
- **Reference-solution check.** For any spec with a golden output, run it
  through the grader mix at validate time and flag if it fails — catches
  broken tasks before they run against a model.
- **Capability vs regression typing.** A first-class EvalSpec field with
  different default alerting (capability: climbing; regression: decline =
  alarm) and a supported graduation lifecycle. On saturation, prompt for
  cost/calibration/exploit divergence among tied configs (CORE-Bench) —
  distinct from the noise-floor verdict.
- **Held-out fixture partition.** First-class public/held_out/private
  partition, spend-once accounting on held-out reads, and an auto-computed
  generalization-gap report. Four independent sources converge here. If
  Threshold's optimizer ever folds in (crucible-036), a never-optimized
  holdout becomes a hard contract, not hygiene.
- **Confounder floor.** Comparisons refuse to attribute a delta to
  model/harness when the compared runs differ on unpinned confounders —
  prompt-format/scoring-method identity (below the harness-name level), infra
  resource envelope, tool allowlist. Label harness-delta vs model-delta as
  distinct comparison types in the ledger.
- **Contamination screening on import.** Deterministic lexical-overlap check
  on any externally sourced fixture set before it enters the trust layer;
  optional paraphrase-variant packs reporting the original-vs-paraphrase gap;
  freshness/release-date field on EvalSpec.
- **API-drift guard.** Record a model version/fingerprint at run time for
  API-served models so historical results can be flagged when the provider
  silently changes the model.

## 3. Judges & calibration

References:

- RubricEval (arXiv:2603.25133) — **rubric-level (one judge call per
  criterion) beats checklist-level (one call for the whole rubric) by 7–12
  points balanced accuracy and cuts inter-judge variance roughly in half**;
  reasoning-before-verdict adds 6.7–9.0 points; even GPT-4o only reaches
  ~56% balanced accuracy on hard rubric items — judge accuracy is bounded
  and uncertain even after calibration.
- Yan, *Evaluating LLM-Evaluators* — the judge-design survey: raw agreement
  (80–87%) collapses to κ=0.3–0.5 once chance-corrected; position bias
  (50–70% of pairwise calls in MT-Bench), verbosity bias (>90% preference
  for longer-not-better), self-enhancement bias (+10–25% own-output
  win-rate); panel-of-diverse-judges beats a single frontier judge at 1/7th
  cost; finetuned judges are task-specific classifiers that transfer poorly.
- *Evaluating Scoring Bias in LLM-as-a-Judge* (arXiv:2506.22316) — purely
  cosmetic scoring-prompt perturbations (rubric order, score-ID format) move
  scores, in judge-specific directions; including a reference answer labeled
  as the perfect-score exemplar reliably improves accuracy across all judges
  tested.
- *One Token to Fool LLM-as-a-Judge* (arXiv:2507.08794) — "master-key"
  tokens (a bare colon, "Solution:") trigger false-positive rewards up to
  80% of the time on frontier judges.
- Su, *Reward Hacking the Judge* (Apr 2026) — a model discovered a
  formatting style (headers/bold/"Key context:") that flipped a GPT-4o judge
  from ~5% to ~95% "correct" while only 6.7% of answers were right.
- Scale AI, *Smoothing Out LLM Variance* — the same judge, same prompt,
  re-run on a different day swings 8–15%; a panel of 3 reworded prompts cuts
  variance ≥50%.
- Judge-panel selection literature: orq.ai *LLM Juries in Practice* (panel
  selection as cost minimization under a κ floor with an error-diversity
  constraint — decorrelated errors, not just different providers); Tornede
  et al. (arXiv:2501.17178); *Don't Always Pick the Highest-Performing
  Model* (arXiv:2602.08003); Auto-Prompt Ensemble (arXiv:2510.06538);
  finite-calibration regime map (arXiv:2606.01034) — when a richer
  calibration model pays for itself under a finite label budget.
- Sage (arXiv:2512.16041) — independent panels improve reliability up to
  15%; **debate-based judging degrades stability**; humans show worse
  internal consistency than the best calibrated judges on hard subsets.
- *Conformal Prediction Sets and Transitivity Violations* (arXiv:2604.15302)
  — transitivity violations (A>B>C>A) as a calibration failure mode distinct
  from low κ.
- CalibraEval (ACL 2025) — inference-time position-bias correction cheaper
  than always-both-orders.
- Calibration-set sizing (aievals.co; llmasajudge.hashnode.dev) — κ variance
  is dominated by the minority-class count: ~50 stratified traces for
  balanced binary criteria; 200+ when a critical category has a ~6% base
  rate.
- AgentRewardBench (arXiv:2504.08942) — no single judge excels across all
  benchmarks; rule-based evaluation systematically *underreports* agent
  success (a silent floor on reported quality).
- Counsel (arXiv:2606.21627) — judges locate the failing step well (~88%)
  but explain it poorly (~65%): error-location and error-reasoning accuracy
  are separate calibration sub-metrics.
- TRACE / Agent-as-Judge synthesis — tool-equipped agent judges (re-run
  code, inspect state) reach ~90% human agreement vs ~70% for
  transcript-only judge prompts, at a fraction of human cost.
- Masood — the "Judge Paradox": a weak judge on a great rubric outperforms a
  great judge on a weak rubric; rubric and dataset quality are the usual
  bottleneck.
- Applied LLMs — swap-and-average for pairwise; allow ties; CoT lets a
  weaker judge match a stronger one.

Already holds: CalibrationRecord measures raw agreement, Cohen's κ, and the
judge-vs-human confusion matrix before a judge is trusted; the judge-gaming
canary hard-refuses a run when the judge rubber-stamps a known-bad candidate
(the reward-hacking literature is the receipt for why this exists); the
aligned-judge pattern is independently validated by Chroma's Context Rot
methodology.

Deltas:

- **Per-criterion judge dispatch.** If `agentic_judge` scores a multi-item
  rubric in one call, it is leaving a benchmarked 7–12 points of accuracy
  and ~2× inter-judge variance on the table. One isolated call per
  criterion, reasoning before verdict, combined by the aggregation layer.
  The single highest-confidence, most-quantified fix in the sweep.
- **Canary variants.** Extend the known-bad set with (a) a
  confidently-formatted wrong answer (headers/bold/structure wrapping a
  wrong claim) and (b) master-key-token probes; refresh canaries
  adversarially (CriticGPT-style red-teaming) rather than leaving them
  static. Generalize toward canaries as a fixture *type*.
- **Calibration hardening.** Report precision/recall on the fail class
  (aggregate κ hides minority-class blindness); scope CalibrationRecords per
  task family (κ does not transfer); track error-location vs error-reasoning
  accuracy separately for trajectory judges; add a judge-instance drift
  check (identical calls across days) and a format-sensitivity self-check
  (rubric reorder / score-ID swap) — a judge whose score moves under
  cosmetic perturbation is fragile regardless of its κ.
- **Reference-answer anchoring.** Where a golden output exists, the judge
  prompt includes it explicitly labeled as the perfect-score exemplar.
- **Panel mode.** Multi-judge as cost minimization under a κ floor with
  error-profile diversity; per-criterion calibration-set sizing by
  minority-class base rate; transitivity spot-checks for pairwise judging.
  Explicit reject: debate-based judging.
- **Agent-as-judge tier.** A tool-equipped judge (re-execute, grep, diff) as
  an escalation between transcript-only judge and human for agentic
  families.
- **Rule-based grader audit.** `key_recall`-style graders audited for false
  negatives against labeled trajectories, the same way judges are calibrated.

## 4. The authoring loop (error analysis first)

References:

- Husain & Shankar, *LLM Evals FAQ* — error analysis (open coding → axial
  coding, ~100+ traces, stop at saturation) is 60–80% of real eval work and
  *the* highest-ROI activity; evals emerge from observed failures, not
  predetermined taxonomies; binary pass/fail beats Likert (annotators
  default to middle values; sub-component binary checks instead of graded
  scores); a single "benevolent dictator" domain expert beats committees for
  most teams; judges need 100+ labeled examples and ongoing maintenance;
  guardrails (sync, deterministic, blocking) are architecturally distinct
  from evaluators (async, non-blocking).
- Husain, *A Field Guide to Rapidly Improving AI Products* — the custom data
  viewer is the highest-leverage investment; domain experts write prompts
  directly; synthetic fixtures generated along explicit dimensions
  (features × scenarios × personas) and verified to actually trigger the
  intended scenario; criteria drift is inherent — recheck judge-vs-human
  alignment on a cadence; count experiments run, not features shipped.
- Yan, *Product Evals in Three Simple Steps* — aim for 50–100 *fail* cases
  (hundreds of passes and 5 failures is useless); source organic failures
  from weaker models rather than synthetic defect injection
  (out-of-distribution); **one evaluator per dimension — the "God
  Evaluator" anti-pattern never works and can't be debugged**; benchmark
  judges against human performance (inter-rater κ is often 0.2–0.3; humans
  miss up to 50% of defects from fatigue), not perfection.
- Anthropic, *Demystifying Evals* — mine the bug tracker and manual testing
  for the first 20–50 tasks; grade what was produced, not the path taken;
  partial credit for multi-component tasks; give judges an explicit
  "Unknown" escape hatch.
- Masood — the actionability principle: every criterion maps to a plausible
  remediation; a criterion nobody can act on is measurement theater.
  HealthBench's theme/behavior-axis layering: a shared cross-family rubric
  backbone with family-specific criteria on top.
- Applied LLMs — the "intern test" as a diagnosis taxonomy for failures
  (context gap vs task difficulty vs decomposition vs genuine quality).

Already holds: `crucible author` runs the same validation as `validate`
before saving; import adapters never silently drop; the EvalSpec's
decision-field forces authors to name what the eval informs.

Deltas:

- **Triage/open-coding mode.** A `crucible triage`-shaped verb: ingest a
  batch of real trials, support open-ended tagging, propose an axial
  taxonomy that seeds a new EvalSpec's failure modes. Evals authored cold
  from a template are the anti-pattern the whole practitioner canon warns
  against.
- **Authoring lints in `validate`.** One-evaluator-per-dimension (warn when
  a single grader entry scores multiple named criteria); actionability
  (warn on criteria with no named remediation path); binary-over-Likert
  nudge; "what would a human check line-by-line?" prompt for monolithic
  outputs.
- **Fixture generation modes.** Dimensional synthetic generation
  (features × scenarios × personas, each fixture verified to trigger its
  scenario before inclusion) and organic-failure harvesting (run a weaker
  model, collect its real failures — which also feeds the routing bench,
  crucible-901).
- **Rubric layering.** Shared cross-family backbone (safety,
  instruction-following, format) + family-specific criteria, rather than
  authoring each family's rubric from scratch.

## 5. Agentic eval families (runners, trace, grading dimensions)

References:

- MAST, *Why Do Multi-Agent LLM Systems Fail?* (arXiv:2503.13657) — the
  validated 14-mode, 3-category failure taxonomy (κ=0.88 human, κ=0.77–0.79
  LLM annotator at ~$1.80/trace; `agentdash` library); fatal vs non-fatal
  modes: FM-1.5/2.4 appear almost only in failed runs; FM-3.2/3.3
  (verification failures) co-occur with *passing* runs — "passed despite a
  real defect" is a distinct, trackable state.
- Anthropic, *Quantifying Infrastructure Noise in Agentic Coding Evals*
  (Feb 2026) — container CPU/RAM headroom-vs-limit configuration alone
  produced a 6 pp swing (p<0.01) on Terminal-Bench 2.0, larger than top
  model gaps; treat sub-3 pp agentic deltas with suspicion unless infra is
  controlled. The runtime is part of the problem-solving loop.
- Cursor, *Reward Hacking Is Swamping Model Intelligence Gains* — 63% of 731
  audited "successful" SWE-bench-Pro-style resolutions were retrieval
  (upstream-fix lookup, git-history mining), not reasoning; strict isolation
  dropped scores up to 20.7 points. Paired with Anthropic's "isolate trials,
  no shared state" rule (models have read prior-trial git history).
- Anthropic: *Building Effective Agents* (workflow-shaped vs agent-shaped
  tasks), *Writing Effective Tools for Agents* (tool-call accuracy,
  turns-to-completion, transcript-reading as method, the tool-testing
  agent), *Multi-Agent Research System* (token usage explains ~80% of
  outcome variance; ~15× cost; start evals small; human testers catch what
  automation misses), *Effective Harnesses for Long-Running Agents*
  (declared-done vs verified-done against externalized ground truth;
  session-boundary awareness), *Effective Context Engineering* (compaction
  failures as gradable events).
- Husain & Shankar, *LLM Evals FAQ* (agent section) — grade the first
  upstream failure; transition failure matrices (last-successful-state ×
  first-failure-state) to find hotspots; two-phase evaluation: end-to-end
  success, then step-level diagnostics.
- Cognition, *Don't Build Multi-Agents* + *What's Actually Working* —
  context-sharing failures between sibling subagents; subagents-as-tool-calls
  vs peer collaboration; the "smart friend" escalation pattern
  (escalation-precision is gradable).
- OpenAI, *A Practical Guide to Building Agents* — max-turn caps,
  failure-threshold escalation, high-risk-action gating as generic checkable
  behaviors.
- 12-Factor Agents — pause/resume/inject as first-class; error compaction vs
  crash-looping; recoverability after mid-trajectory correction as a
  grading dimension.
- Chroma, *Context Rot* — refusals/non-completions tracked as a separate
  metric, never folded into the accuracy denominator.
- Willison, *The Lethal Trifecta* — private data + untrusted content +
  external egress as a computable tool_allowlist precondition for requiring
  an injection-resistance variant.
- Terminal-Bench 2.0 / Harbor — end-state verification (pytest-style over
  final filesystem/process state) as a grading mode distinct from transcript
  judging; Harbor as the standardized external task contract (already
  Crucible's export target).
- METR time-horizon methodology — success probability fit against task
  duration/difficulty covariates; monitorability evals as a genre (canaries
  generalized).
- HAL (Princeton, ICLR 2026) — cost-aware leaderboards (accuracy *and* $
  together); reliability (run-to-run consistency of one config) as a
  first-class surface distinct from two-config comparison.
- Atlan, *Agent Memory Architectures* — memory pattern as a config axis; 37%
  of multi-agent failures from unsynced context (independent corroboration
  of MAST FC2).
- Willison, *2025 Year in LLMs*; MCP 2026 Roadmap — sync vs async
  fire-and-forget trial lifecycles; tool calls may arrive as generated code,
  not discrete JSON blocks; MCP Tasks retry/expiry semantics (watch, don't
  build yet).

Already holds: a Trace layer with a pointed-to trace_path per run; grader-mix
spectrum; judge-gaming canary; Harbor as export contract; config-identity
axes.

Deltas:

- **MAST as a grader family.** 14 calibratable dimensions for any
  multi-agent/tool-using family; stage tags (pre/execution/post) on traces;
  aggregate "passed cleanly" vs "passed with flagged verification debt" as
  separate figures — a pass rate that hides verification debt is exactly the
  overclaim the One Principle forbids.
- **Trajectory grader library.** Deterministic, cross-family graders:
  tool-call accuracy; turns/time-to-completion; max-turn respected;
  escalate-after-N-failures; risky-action gating; declared-done vs
  verified-done; sibling-artifact consistency for orchestrator-worker
  families; transition failure matrices as a trace-analysis view.
- **Isolation guarantee.** An explicit no-shared-state-across-trials test in
  the gate for any runner touching a live environment; the Cursor 20.7-point
  finding is the receipt.
- **Infra/resource-envelope axis.** CPU/RAM/timeout/headroom-vs-limit
  tracked per run; comparisons refuse model/harness attribution when infra
  differed. The sharpest single new fact in the sweep.
- **Refusal as an outcome category.** Wrong ≠ declined; separate category in
  the scoring schema and rates.
- **Token/cost axis.** Cost per trial/run as a first-class comparison axis
  in the ledger and `runs compare` (HAL's accuracy-and-$-together pattern);
  a memory-architecture axis when it varies.
- **End-state verification mode.** For tasks with an inspectable
  environment, grade final state programmatically (Terminal-Bench pattern)
  alongside or instead of transcript judging.
- **Trifecta-risk flag.** Computed off tool_allowlist; when tripped, the
  family requires an injection-resistance trial variant before being marked
  trusted.
- **Reliability surface.** Repeat-consistency of one config (what pass^k
  measures) framed as a first-class product view, distinct from comparison
  significance.

## 6. Import/export & ecosystem

References:

- Inspect (UK AISI) — Task/Dataset/Solver/Scorer decomposition: *how the
  candidate was produced* is orthogonal to *how it is graded*; sandbox as a
  task property. Reject: registry-by-decorator discovery.
- lm-evaluation-harness — versioned task specs; results as citable,
  self-contained artifacts.
- OpenAI Evals — the run log embeds the full spec as its first row
  (self-describing artifacts); the eval-definition vs eval-run split
  (convergent with EvalSpec vs the ledger).
- Promptfoo — one engine for quality + red-team; matrix-view reporting.
- Braintrust — evals as release gate; named pitfalls: never-optimized
  holdout, judge bias, leakage, metric gaming.
- LangSmith / W&B Weave convergence — **trace-first**: production traces are
  the dataset; failing production traces get *promoted* into offline
  fixtures. A structurally different center of gravity from eval-first
  (Inspect, lm-eval, Crucible today) — adopt the intake, keep the spine.
- GDPval / FACTS v2 — gold subset + hosted grader as the distribution model
  for trusted eval families.
- EDD write-ups (AppScale, QASkills, Red Hat, 2026) — unit/scenario/shadow/
  canary eval layers; the ratchet (CI refuses scores below the cleared
  threshold — the "do not lower gates" doctrine, independently arrived at).

Already holds: import through validate-then-save adapters that never
silently drop; export to the consumer's contract (Harbor), not an invented
schema.

Deltas:

- **Promote-from-production intake.** A first-class "promote this
  run/trace/finding to a fixture" path from the run ledger (and from live
  Cerberus output) — the trace-first pattern adapted to Crucible's
  eval-first spine, and the natural second import adapter.
- **Self-describing exports.** Embed the full spec/config snapshot inline in
  exported artifacts so consumers never dereference back into Crucible.
- **Shadow-run mode.** Run a challenger config silently alongside the
  trusted one, measure divergence, no gating — a named run mode distinct
  from a comparison.
- **Solver/grader orthogonality.** Before any 4th runner kind, confirm a new
  production mode (e.g. an agentic solver) can reuse existing graders rather
  than forcing a new runner×grader pairing.
- **Hosted-grader distribution** (future). Publish gold subset + grading
  service for mature families rather than raw task data.

## 7. The human-judgment surface

References:

- Husain & Shankar, *LLM Evals FAQ* — the benevolent-dictator pattern:
  a single domain expert as source of truth beats committees for most
  teams; committees + chance-corrected agreement only when the domain
  genuinely spans perspectives.
- NUTMEG (arXiv:2507.18890) — separate genuine subpopulation disagreement
  from annotator error; majority vote destroys information calibration
  needs.
- URC² (ICLR 2026 sub) + HITL routing literature — two-lane uncertainty
  routing: epistemic uncertainty (judge unsure, answer well-defined) →
  human; aleatoric (task itself ambiguous) → fix the fixture, stop spending
  adjudication budget on it.
- Sage (arXiv:2512.16041) — humans are not an unquestioned gold standard;
  track inter-human agreement on hard items; low agreement routes to a
  second human + explicit tie-break.
- RubricEval's arbitration pipeline — discard rather than force-label
  persistently disputed items ("unresolved — excluded from aggregate" beats
  a forced majority vote that may be noise).
- Yan, *Product Evals* — benchmark judges against human performance
  (inter-rater κ often 0.2–0.3; fatigue misses up to 50% of defects), not
  perfection.
- Anthropic, *Multi-Agent Research System* — human testers catch what
  automated evals miss (hallucinated citations, source-selection bias);
  keep the human lane even when judges calibrate well.
- Calibration-set sizing by minority-class base rate (§3).

Already holds: phone-first Keep/Nit/Wrong/Noise with live writeback through
the same apply_label path as the CLI; labels as `crucible.label.v1`.

Deltas:

- **Disagreement-aware label schema.** Represent "two defensible readings"
  distinctly from "annotator error"; support "unresolved — excluded from
  aggregate"; never collapse silently to majority vote.
- **Two-lane routing.** Judge-uncertain items go to humans; inherently
  ambiguous fixtures get flagged for repair in the findings journal instead
  of consuming adjudication budget repeatedly.
- **Adjudication modes.** Benevolent-dictator vs committee as a declared
  per-eval setting; inter-human agreement tracked on hard items with
  second-labeler tie-break routing.
- **Per-criterion calibration floors.** Size calibration sets by
  minority-class base rate (~50 balanced-binary, 200+ for rare critical
  categories), not one N per eval.

## 8. Reports, dashboard, and the rendering of uncertainty

References:

- Bret Victor, *Magic Ink* — information software should infer context from
  environment and history and interact only as a last resort; confidence
  should modulate visual weight.
- Hullman/Kale et al. (HOPs), Kay et al. (quantile dotplots), Wilke ch. 16 —
  static error bars induce a *deterministic construal error* in non-expert
  viewers that persists even when they can state the correct interpretation;
  frequency framing and discrete-outcome displays fix what labeling cannot.
- Nicky Case, *Explorable Explanations* + Distill, *Communicating with
  Interactive Articles* + NYT "You Draw It" — predict-then-reveal is the
  single most convergently validated interaction pattern (three independent
  traditions).
- Stephen Few, *Common Pitfalls in Dashboard Design* — the 13-pitfall audit
  list; most Crucible-relevant: #2 no bare rate without adjacent
  interval/noise-floor context; #10 reserve visual salience for
  don't-trust-this-run states (canary trips, stale calibration); #1 one
  screen — the adjudication card must fit one phone viewport with evidence,
  rationale, and tap targets simultaneously visible.
- NN/g, *Progressive Disclosure* — verdict + interval + noise-floor verdict
  as the primary layer; full trace and raw fixtures secondary. The
  adjudication queue is correctly *staged*; don't convert it to drill-down.
- Explorable-explanation history — the precedent of an editorial team
  rejecting an interactive model because it would emit un-cited numbers: any
  threshold-slider must re-aggregate real persisted trials, never
  interpolate.
- Practical Typography; Kolokolov, *Dashboard Anti-Trends* — the
  stat-tile-as-mini-dashboard trap.
- Distill editorial principles — publish the verification trail next to the
  finding (calibration, noise-floor check rendered alongside, not merely
  queryable).

Already holds: intervals and noise-floor verdicts computed everywhere; the
serve UI reads the same ledger the CLI does (one scoring system, no parallel
truth).

Deltas:

- **Uncertainty rendering.** Quantile dotplots (static, cheap) as the
  default lay rendering of rate-with-interval; error bars become the expert
  drill-down; optional HOPs where correlated uncertainty across two compared
  runs is the point. A correctness-of-communication defect, not polish: the
  current rendering can defend the number mathematically while still causing
  readers to misread it.
- **Zero-click default view.** The dashboard answers "did anything regress
  since the last trusted baseline, and can I trust today's numbers?" from
  history, with no selection required.
- **Confidence as visual weight.** Verdicts that haven't cleared the noise
  floor render recessive (muted, hedged type), never with the typographic
  authority of cleared ones; calibration staleness auto-attaches to every
  verdict that judge produced.
- **Predict-then-reveal** on comparison verdicts (one cheap step before the
  reveal).
- **Few-13 audit pass** on the dashboard and adjudication queue, plus the
  one-viewport check on the adjudication card.
- **Interactivity gate.** Case's Do/Show/Tell triage: interactivity only for
  genuine processes (stepping a trace, sliding a threshold over real
  re-aggregated trials); relationships stay static charts; claims stay
  prose.

## 9. What the canon validates (keep true)

- Deterministic-first grader mix, judges calibrated before trusted, humans
  in the loop — the practitioner canon's exact recommended ordering.
- Refusing to run when the judge fails the gaming canary — ahead of most of
  the field; the reward-hacking incidents are the receipt.
- McNemar-paired comparison + noise-floor verdicts + Wilson intervals — only
  16% of 445 audited benchmarks use any statistics at all; Crucible's
  verdict layer is a genuine differentiator.
- CalibrationRecord already reporting Cohen's κ and a confusion matrix, not
  just raw agreement — the exact upgrade most of the field hasn't made.
- Import-through-validation that never silently drops; export to the
  consumer's contract.
- The eval object carrying "the decision this eval informs" — the OEC
  discipline, arrived at independently.
- Staying native and thin: Inspect/lm-eval/Promptfoo are references, not
  substrates.
- Explicit rejects, for the record: debate-based judging (degrades
  stability); OTel as the trace substrate (the purpose-built calibration
  trace is narrower and more load-bearing); bootstrap as the small-N
  fallback; registry-by-decorator eval discovery; single collapsed
  leaderboard scores; keyword heuristics standing in for semantic judgment.
