# 036 - Interval should widen for measurement error, not just sample size

Status: open
Priority: someday (operator: "not super crucial ... something we maybe want to
think about")

## Problem

The reported score interval is a Wilson binomial CI. It models exactly one
source of uncertainty — **sampling variability** under an i.i.d.-Bernoulli,
single-shared-`p` assumption — and is structurally blind to everything that
makes an eval *good*. Consequence, in the operator's words: "100 poorly defined
tests don't give you the same confidence that 30 well designed tests do," yet
Wilson reports a *tighter* interval for the 100. The tightness is real but it is
tightness around possibly-the-wrong-number. This is in tension with Crucible's
one principle (refuse to report a delta it cannot defend): today the interval
defends against sample size and nothing else.

What Wilson cannot see:

- **Redundancy / non-independence.** 100 tasks that are really 10 tasks copied
  10x have an effective sample size near 10, not 100. Wilson sees only `k/N` and
  reports a falsely tight interval. Classic *design effect*:
  `n_eff = n / (1 + (m-1)*rho)`.
- **Label / grader error.** A wrong rubric or mislabeled ground truth yields a
  tight interval around a biased estimate — confident garbage.
- **Difficulty spread / discrimination.** 30 items spanning the capability space
  carry more information than 100 clustered in one easy corner, but can produce
  identical Wilson intervals. Wilson measures precision of the sample mean, not
  representativeness of the sample.
- **Over-dispersion.** Heterogeneous item difficulty makes a plain Binomial
  understate variance; the true interval is wider than Wilson admits.

## Direction (candidates, roughly in leverage order)

1. **Effective sample size / design effect.** Detect item redundancy
   (near-duplicate prompts/expectations, or cluster structure) and inflate the
   interval via `n_eff`. Biggest, most tractable win; directly encodes "100
   redundant tests approx far fewer real tests."
2. **Grader-calibration propagation.** `CalibrationRecord` already computes judge
   agreement / Cohen's kappa / fail-class precision-recall. Fold that reliability
   into the reported interval so an eval graded by a 90%-reliable judge cannot
   claim >90%-grade certainty. Distinctive move — the calibration layer exists;
   it just is not wired into the CI. Deterministic exact-match graders are the
   reliability=1.0 special case.
3. **Item information / coverage (IRT-flavored).** Report test information and a
   construct-coverage score *alongside* the interval (not folded into a fake
   scalar — the reader wants the information, not a false precision number), so a
   tight interval on a narrow eval is flagged.
4. **Beta-Binomial / hierarchical model.** The principled generalization of
   Wilson for heterogeneous-difficulty items (between-item variance / partial
   pooling).
5. **Extend prospective power gate.** `min_effect_of_interest` /
   `required_n_paired` already warn on too-small N; add an item-quality warning
   (redundant / non-discriminating items), not just a count warning.

## Oracle

- [ ] `crucible-core::measure` grows an effective-sample-size / design-effect
  adjustment with unit tests showing a redundant corpus reports a wider interval
  than its raw N implies, and a diverse corpus of the same N does not.
- [ ] The path from `CalibrationRecord` reliability into the reported interval is
  designed (even if only wired for the agentic-judge runner first), with a test
  that a lower judge kappa widens the interval.
- [ ] Decision recorded on what stays a companion signal (coverage, information)
  vs. what folds into the interval (n_eff, grader reliability, over-dispersion) —
  do not collapse construct validity into something that looks like precision.
- [ ] No regression to the deterministic exact-match path: reliability=1.0,
  independent items => today's Wilson interval, unchanged.

## Notes

Raised 2026-07-07 in the operator walkthrough after a 3/3 tracer run reported
`[43.8%, 100%]`. The 43.8% floor is correct *sampling* math; this card is about
the other axes of uncertainty the number does not yet carry.
