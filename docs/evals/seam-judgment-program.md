# Seam Judgment Experimental Program

Status: research and design, 2026-07-13

## The decision

This program decides when an agent configuration may propose or implement
software boundaries without mandatory seam-placement review. It measures one
capability:

> Given a real, imperfect implementation, can the agent put semantic judgment,
> declared variability, and deterministic enforcement in the right places and
> leave a working system with no boundary inversion?

The score must be able to change model routing, prompt choice, harness choice,
or primitive composition. A broad table that changes several of those axes at
once is not evidence.

## What v0 taught us

`seam-judgment-v0` is a valid doctrine-comprehension smoke test. It is not a
routing benchmark. Its system prompt recites the placement rule; its scenarios
have one dominant cue; DeepSeek scored 23/24 and GLM scored 24/24. Adding more
models or 157 paraphrases would measure saturation more precisely without
testing architecture judgment.

The first run also showed why elicitation failures must be separated from
capability failures: an 80-token cap produced 37 empty or truncated outputs.
Only the corrected 1,000-token run is model evidence.

## Ideation result

Method: **SCAMPER**, created by Bob Eberle in 1971 from Alex Osborn's
brainstorming checklist.

The obvious expansions were rejected:

- run the same easy labels across twenty models;
- replace the exact grader with an uncalibrated prose judge;
- combine every model, prompt, harness, and primitive in one factorial table.

The productive SCAMPER transformations were:

- **Substitute:** prose scenarios become runnable microrepositories derived
  from real corrective history.
- **Eliminate:** remove the doctrine recital, answer labels, project names,
  and clues that name the mutation.
- **Reverse:** instead of asking “model or deterministic?”, give the agent a
  plausible wrong-layer implementation and ask it to repair the system.

This produces two related evals, not one compromised hybrid:

1. **Seam diagnostic:** a cheap prompt benchmark for task bring-up, prompt
   sensitivity, and broad model probing. It never supports agent-routing claims.
2. **Seam agency:** a sandboxed patch benchmark where agents inspect context,
   change code, and face executable verification. This is the decision-grade
   benchmark.

## The three-layer model

`DECLARATION` is not a flat third class beside `MODEL` and `DETERMINISTIC`.
Declarations may configure semantic judgment or exact mechanisms. Each task is
therefore scored along three dimensions:

- **Judgment:** semantic meaning is handled by a model, or the task genuinely
  requires no semantic judgment.
- **Structure:** variability that belongs in data or declarations does not grow
  as imperative branch mass.
- **Enforcement:** policy, persistence, approval, parsing, limits, and other
  must-fire behavior remain deterministic around model output.

Headline success is still binary: the repaired system passes every required
observable invariant. Dimension scores explain why configurations fail; they
do not average a policy bypass into a partial pass.

## Corpus architecture

Build 36–48 decision tasks only after a seven-task development set proves the
task and verifier shape. Balance by seam conflict, not by a toy output label:

| Family | What it tests |
|---|---|
| Semantic judgment + deterministic gate | Model interprets intent; code enforces permissions, disclosure, spend, or destructive-action policy. |
| Semantic judgment + exact consumer | Model proposes; typed parsing, refusal/retry, stable identity, and persistence surround it. |
| Declaration + model judgment | New semantic categories or routing evidence extend through declarations rather than keyword/branch tables. |
| Declaration + deterministic mechanism | Versions, gates, capabilities, or transitions extend through declarations consumed by exact code. |
| Pure semantic control | No trust boundary; regex/enums over natural language are the defect. |
| Pure deterministic control | Timeout, lease, checksum, authorization, or atomicity; adding a model is the defect. |
| Mixed decomposition | One requirement needs all three layers. These are the hardest and most decision-relevant tasks. |

An `INSUFFICIENT_INFORMATION` class belongs in the cheap diagnostic set. Patch
tasks use it only when the environment supports an observable clarification
outcome; otherwise hidden requirements would make the task broken.

### Source extraction

1. Deterministically collect the merged corrective diff, parent snapshot,
   card/issue, review comments, and final tests.
2. Use a model—not keyword filters—to screen for placement corrections.
3. Have a human curator confirm the causal seam and write a private provenance
   receipt.
4. Slice the smallest 100–300-line buildable module.
5. Rename domain entities and perturb incidental syntax while preserving the
   invariant.
6. Remove original history, identifiers, network access, and original tests
   from the agent sandbox.
7. Keep every variant from one source incident in the same corpus cluster.

Initial sources should include Gazette semantic heuristics, Crucible
publication, path confinement, Roster capability routing, Powder
dispatchability/leases, comparison attribution, and the Bridge hint-array
correction.

### Split and renewal

- Development: 12 public tasks with references, used for harness bring-up.
- Calibration: 12 private tasks used for verifier and judge calibration.
- Test: 24–36 private frozen tasks used for comparisons.
- Renewal: post-cutoff corrective diffs added quarterly; active references are
  never published.

Split by source incident and repository family, not transformed variant.
Repeated trials and sibling variants are clustered observations, not extra
independent tasks.

## Gold packet and task qualification

Every task has a private gold packet:

- source receipt and causal summary;
- observable invariants and forbidden outcomes;
- reference patch plus one structurally different acceptable patch;
- hidden verifier and mutation suites;
- expected judgment/structure/enforcement map;
- ambiguity ruling and known verifier limits.

Two experts independently author the seam map and invariants before seeing
model outputs. A third adjudicates disagreement. A task leaves the headline set
if experts cannot agree on observable behavior.

Reference and alternative patches must pass. Each task must reject at least two
plausible wrong-layer implementations. This directly addresses recent coding-
benchmark audits that found underspecified prompts, overly strict tests, and
low-coverage graders can dominate the result.

## Verifier ladder

### 1. Deterministic headline

- build and behavioral acceptance tests;
- adversarial fake-model responses;
- no unauthorized state mutation;
- invalid model output refuses or retries safely;
- declaration extension without control-flow edits where applicable;
- exact operations remain model-independent;
- bounded time and resources.

Grade the final state, not a prescribed tool sequence.

### 2. Tracked deterministic dimensions

Record whether the model is used only on semantic inputs, gates are independent
of model output, raw output is parsed before use, and new cases extend through
declarations rather than branch edits. These should explain headline failures
without becoming a brittle trajectory rubric.

### 3. Calibrated model judge

Initially non-headline, limited to architectural coherence and shallow-wrapper
detection. Use a different family from the worker, blinded human labels,
fail-class precision/recall, `UNKNOWN`, format-sensitivity and drift probes,
and reference exemplars. Pairwise judgments are order-swapped.

### 4. Human audit

Inspect every deterministic/judge disagreement, every novel passing structure,
and a random 20% of pilot passes and failures. Continue transcript review until
the failure taxonomy stops changing.

### Required mutations

The suite kills applicable mutations that:

- replace model judgment with keyword or regex branches;
- put must-fire policy only in a prompt;
- let raw model output control filesystem, persistence, approval, or parsing;
- hard-code a branch table instead of consuming a declaration;
- add a model to an exact deterministic operation;
- accept malformed or unknown model output;
- collapse rich semantic context into an enum before judgment;
- explain the right architecture without changing behavior;
- inspect forbidden paths or overfit visible tests;
- preserve the original inversion behind a shallow wrapper.

## Seven-task qualification set

Build these before a broad matrix:

1. **Publication boundary — mixed seam.** Starter asks a model whether fields
   are safe, then writes its approved object. The repair must allow semantic
   disclosure advice while deterministically refusing credential shapes and
   undeclared fields, validating the packet, and writing atomically.
2. **Incident grouping — semantic + exact consumer.** Replace token overlap
   with meaning-aware comparison; validate stable group IDs, persist
   idempotently, and refuse malformed output.
3. **Provider routing — declaration + judgment + gate.** Replace provider-name
   branches with declared capabilities; let a model rank eligible candidates;
   enforce budget and required-tool filters before selection.
4. **Comparison attribution — declaration + deterministic.** Add a new identity
   axis through declaration and exact comparison without another special-case
   branch.
5. **Memory extraction — pure semantic control.** Remove keyword heuristics
   without inventing a schema or policy engine that destroys context.
6. **Claim lease — pure deterministic control.** Repair exact lease ownership
   and expiry without calling a model.
7. **Operator action router — mixed seam.** Model classifies the requested
   action and missing context; deterministic approval/persistence boundaries
   prevent send, publish, buy, or destructive effects without authority.

Qualification order: reference patches, acceptable alternatives, named
mutants, then agent attempts. Do not use agent scores to debug an unqualified
task.

## Experimental matrix

Run each stage only if the prior stage produces useful variance. Model catalog
facts and prices must be refreshed on dispatch day.

### Stage A — cheap model probe, one-shot diagnostic

Hold prompt/config fixed. Start with five different families: DeepSeek V4
Flash, MiniMax M3, Qwen 3.7 Plus, GLM 5.2, and Kimi K2.7 Code. Thirty-six easy
diagnostic tasks would cost well under one dollar at the 2026-07-13 catalog
rates, but this stage exists to catch saturation and broken formatting, not to
rank agents.

Stop if every family remains above 90%; fix the corpus before frontier spend.

### Stage B — articulation probe

Hold DeepSeek V4 Flash and every request parameter fixed. Compare:

- no doctrine;
- a minimal placement question;
- the full worked doctrine.

This measures prompt sensitivity. Crucible currently lacks first-class
`prompt_delta` attribution, so record the prompt hash and do not call the result
a model delta. Run each arm once during bring-up, with a total Stage B cap of
$1. Stop before inference if the catalog-price estimate exceeds the cap.

### Stage C — replication and frontier escalation

The provisional routing threshold is 80% task success. “Near” means that a
configuration's 95% interval overlaps 80%, or that two candidates are within
ten percentage points and the choice would change routing. Repeat every near
configuration three times on the same tasks. Use repeated trials to estimate
consistency, not to inflate independent `n`. Escalate to Grok 4.5, Claude
Sonnet 5, and GPT-5.5 only if cheap models expose a real gradient. Stage C is
capped at $10; refresh response-model identity and a catalog-price estimate
from the declared input/output limits before every dispatch.

### Stage D — reasoning effort

Hold model, prompt, and runner fixed; compare supported reasoning levels. This
is blocked today: Crucible does not send or identity-hash reasoning effort for
`prompt_benchmark`. Add that axis before making a trusted comparison. Once
unblocked, run low/medium/high effort three times each and cap the pilot at
$10.

### Stage E — real harness probe

Hold model, task snapshot, prompt, resource envelope, tools, and budget fixed.
Begin with 12 diagnostic patch tasks across raw API, Pi bare, Goose minimal,
and OpenCode pure. Pi is the first primitive-ablation harness because its CLI
can disable context, extensions, skills, and prompt templates independently.
Run every harness/task pair three times (`k = 3`), report pass@3 and pass^3,
and treat the source task—not each trial—as the independent unit. Before
launch, price the maximum input, output, and supported reasoning allowance for
all 144 trials; Stage E has a hard $150 cap and stops when 80% is consumed so
failed or retried jobs cannot silently overrun it.

This is also blocked from honest execution through `prompt_benchmark`:
`harness` and `tool_allowlist` are recorded identities, not provisioned
behavior. `harbor_task` runs a real sandboxed agent, but each external harness
needs a versioned adapter/agent contract and complete transcript/config receipt.
Never relabel a direct OpenRouter call as a Pi or Codex run.

### Stage F — primitive ablation

Freeze one Pi baseline: fresh sandbox, task prompt, required repository files,
shell/edit tools, one worker, no optional skill, no retained memory, and no
post-work critic. Toggle exactly one treatment against that baseline; do not
make these cumulative arms:

1. **Context:** baseline task-required files only versus an additional declared
   repository doctrine/context packet.
2. **Tools:** the same task with inspection-only tools versus inspection plus
   bounded edit/test tools. Use tasks where both outcomes can be scored.
3. **Skill:** no optional skill versus the declared model-native-first skill.
4. **Memory:** a fresh worker with no session summaries or retrieved memories
   versus the same worker plus one frozen, task-relevant memory packet. The
   packet, retrieval query, and bytes are versioned artifacts; ordinary
   repository files are not called memory.
5. **Critique:** one worker's final patch versus the same worker receiving one
   blinded independent review and one bounded revision turn. The reviewer
   cannot edit or delegate.
6. **Multi-agent composition:** one worker versus a coordinator with two
   declared roles—builder and verifier—sharing the same total model-token,
   tool, time, and revision budget. This arm has no extra critic or memory.

Every primitive needs tasks where it has a causal opportunity to help. Run
each task/arm three times (`k = 3`), report pass@3 and pass^3, and cluster by
source task. A subagent cannot earn credit on a two-line classification task.
Stage F has a hard $200 cap and the same 80%-consumption stop. Record the full
system prompt, model, effort, tools, memory packet, workspace revision,
resource limits, role topology, transcript, and cost for every arm.

### Spend ledger

No stage starts from prose estimates alone. Its dispatch receipt must record
the current catalog timestamp, per-token or per-run prices, maximum input and
output tokens, reasoning allowance, trial count, expected cost, hard cap, and
actual accumulated cost. The pilot caps are A $1, B $1, C $10, D $10, E $150,
and F $200: $372 maximum, with E/F stopping at 80% of cap. A stage is
re-estimated after any model, prompt-size, task-count, reasoning, retry, or
harness change. The existing 2x-estimate stop may fire below the cap; the hard
cap always wins.

## Statistical contract

- Pair every comparison on identical tasks and change one axis.
- Report intervals, paired noise-floor verdict, response-model identity, cost,
  and resource envelope.
- Use 36 tasks as diagnostics only. A defensible paired difference near ten
  percentage points will likely require roughly 150–250 genuinely distinct
  cases depending on observed discordance; let Crucible calculate the final
  requirement.
- Do not treat seeds or variants from one source incident as independent.
  Crucible does not yet compute cluster-aware intervals, so report that gap.
- For agent consistency report both pass@k and pass^k when the operating
  decision cares about “can ever solve” versus “reliably solves.”

## Stop and falsification conditions

Pause or redesign when:

- experts cannot agree on invariants;
- a reference or structurally different valid patch fails;
- a named mutation survives;
- visible instructions omit a hidden requirement;
- failures come from sandbox, network, timeout, or token limits;
- all cheap models or three families across two harnesses score at least 95%;
- cheap prompt performance nearly perfectly predicts patch success;
- provider response-model drift occurs within a comparison;
- a harness receipt omits actual prompt, model, effort, tools, workspace,
  limits, transcript, or cost;
- hidden reasoning makes spend exceed the estimate by more than 2x.

## Current Crucible gaps

Before the full program, Crucible needs:

- a real external-agent runner/adaptation contract for Pi, Goose, OpenCode,
  Codex, and Claude rather than identity-only harness labels;
- reasoning effort as an applied and config-identity axis;
- prompt/articulation attribution distinct from generic `config_delta`;
- imported harness transcripts and end-state evidence;
- cluster-aware uncertainty or an explicit grouped-analysis export;
- held-out/spend-once corpus governance.

The seven-task Harbor qualification pilot can start before all of these land,
but no cross-harness or grouped-population claim may outrun them.

## Research source matrix

| Lane | Status | Contribution |
|---|---|---|
| Codebase | Complete | v0 receipts; environment identity-only limitation; Harbor execution; existing calibration and comparison machinery. |
| Roster/model routing | Complete | Current model pool, harness surfaces, local versions, dated catalog costs, and Pi as the cleanest ablation substrate. |
| Primary methodology | Complete | Agent eval construction, scaffold/elicitation effects, sandbox end-state scoring, and benchmark-quality failure modes. |
| Agentic acquisition | Complete | AI-scout model/harness matrix and independent corpus/verifier design. |
| Extraction | Skipped | No new site crawl was needed after primary pages and local sources resolved the design. |
| Recency/discourse | Skipped | Social sentiment would not change the execution or verifier contract. |
| Paid inference | Skipped | Task qualification must precede broad spend. |
| Repo-aware critique | Complete | Fresh critic rejected conflated primitive arms, uncapped later stages, and vague replication; the revised contracts passed re-review. |

Primary external references:

- [Anthropic: Demystifying evals for AI agents](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents)
- [OpenAI: Separating signal from noise in coding evaluations](https://openai.com/index/separating-signal-from-noise-coding-evaluations/)
- [OpenAI: Introducing SWE-bench Verified](https://openai.com/index/introducing-swe-bench-verified/)
- [METR: Guidelines for capability elicitation](https://evaluations.metr.org/elicitation-protocol/)
- [METR: Example autonomy evaluation protocol](https://evaluations.metr.org/example-protocol/)
- [Inspect: Multiple scorers and sandbox access](https://inspect.aisi.org.uk/multiple-scorers.html)
- [OpenRouter model catalog](https://openrouter.ai/api/v1/models)
- [OpenRouter reasoning controls](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)
