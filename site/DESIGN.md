# Crucible DESIGN.md

This file is the product's public-site brand contract. Keep it short and exact:
agents and humans should be able to update `site/` from this file without
inventing a second design system.

## Brand Voice

- Rigorous and statistically honest, skeptical of hype: mirror the README's
  own register, "refuse to report a delta it cannot defend."
- Lead with the noise-floor refusal itself, then the mechanics (Wilson
  intervals, calibration, canaries) that make the refusal credible.
- No marketing fog. No "AI-powered" filler. State exactly what a rate does
  and does not prove.
- Show the boring case (`inside_noise_floor`, empty ledger) as proudly as the
  exciting one — an eval tool that only shows wins is not trustworthy.

## Pitch One-Liner

`Crucible is the eval and benchmark workbench that refuses to report a delta it cannot defend — every rate carries a confidence interval, and rank gaps inside the noise floor come back inconclusive, not a winner.`

## Lucide Mark

- Icon: `flask-conical`
- Reason: reused from the live Crucible workbench sidebar (`crucible serve`
  renders this exact Lucide mark next to "CRUCIBLE" in `crucible/src/serve.rs`)
  — it is already the product's mark, and a flask fits an eval arena that
  measures and titrates model behavior rather than just running prompts.
- Rule: the mark is an inline Lucide SVG inside `.ae-app-mark`. No bespoke
  marks, logo images, emoji marks, or colored wordmarks.

## Palette Hooks

Root pin: `data-ae-theme="violet"`. Violet reads as analytical/judicial
(calibration, verdicts) and stays clear of Powder's ultramarine blue.

```css
:root[data-ae-theme='violet'] {
  --ae-accent: #6d28d9;
  --ae-accent-dark: #c4a3ff;
}
```

No extra categorical hues needed yet — the workbench itself is monochrome
plus a pass/fail pair, and the site should not invent a palette the product
UI doesn't have.

## Screenshot Inventory

All three captures are real screenshots from a live `crucible serve` instance
(port 4174) reading the actual SQLite run ledger at
`runs/local/crucible-101/final.sqlite`, seeded by the `docs/operator-walkthrough.md`
five-task benchmark run against two real OpenRouter models
(`deepseek/deepseek-v4-flash`, `z-ai/glm-5.2`). No mockups.

| File                                            | Surface                    | State                                                     | Caption                                                              |
| ------------------------------------------------ | --------------------------- | ---------------------------------------------------------- | --------------------------------------------------------------------- |
| `site/assets/screenshots/01-noise-floor-verdict.png` | Comparison view             | Live paired McNemar result, `inside_noise_floor`          | The pitch, live: a real score gap that Crucible refuses to call a win. |
| `site/assets/screenshots/02-benchmark-library.png`   | Benchmarks list (home view) | Real specs from `evals/`, one card showing a stored result | The declared benchmark library, not a demo dataset.                   |
| `site/assets/screenshots/03-run-receipt.png`         | Receipts / run detail       | Stored run with per-task pass/fail and Wilson range         | The audit trail: every task's verdict, latency, and cost.             |

## Footer Links

- Misty Step: `https://mistystep.io`
- GitHub: omitted — `misty-step/crucible` is a private repository; add this
  link back only after the repo goes public.
- Weave: omitted — Crucible is not a Weave-family product surface.

## Release Notes Rule

`site/changelog.html` is user-facing. Write entries as product outcomes, not
commit logs. Each entry needs a date, a version or release label, and one or two
plain-language bullets.
