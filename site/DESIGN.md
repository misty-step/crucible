# Crucible DESIGN.md

This file is the product's public-site brand contract. Keep it short and exact:
agents and humans should be able to update `site/` from this file without
inventing a second design system.

## Brand Voice

- Rigorous and statistically honest, skeptical of hype.
- Lead with the locked public tagline, then support it with concrete mechanics
  on the features page only: Wilson intervals, calibration, canaries, and
  noise-floor verdicts.
- No marketing fog. No "AI-powered" filler. State exactly what a rate does
  and does not prove.
- Show the boring case (`inside_noise_floor`, empty ledger) as proudly as the
  exciting one — an eval tool that only shows wins is not trustworthy.

## Fleet Site Lock

- Lock: operator lock-in 2026-07-07, `misty-step-936`.
- Homepage H1, exact: `Design evals. Discover winners.`
- Layout: Mural.
- Homepage structure: one full-viewport hero only, no scroll.
- Hero image: `site/assets/hero.jpg`, copied from the locked production asset
  `crucible-hero.jpg`; generated with `gpt-image-1` in the Misty Step fresco
  language.
- Hero opacity: `0.35`.
- Hero copy: frosted paper panel anchored lower-left; panel contains only the
  H1 and `Get started` CTA.
- Header nav: `features`, `get started`, `changelog`, `github`.
- Footer: mode toggle on the left; right side reads `a Misty Step project`
  with `Misty Step` linked to `https://mistystep.io`, followed by an inline
  GitHub glyph linked to `https://github.com/misty-step/crucible`.

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
| `site/assets/screenshots/01-noise-floor-verdict.png` | Comparison view             | Live paired McNemar result, `inside_noise_floor`          | The proof surface: a real score gap reported as inconclusive.          |
| `site/assets/screenshots/02-benchmark-library.png`   | Benchmarks list (home view) | Real specs from `evals/`, one card showing a stored result | The declared benchmark library, not a demo dataset.                   |
| `site/assets/screenshots/03-run-receipt.png`         | Receipts / run detail       | Stored run with per-task pass/fail and Wilson range         | The audit trail: every task's verdict, latency, and cost.             |

## Footer Links

- Misty Step: `https://mistystep.io`
- GitHub: `https://github.com/misty-step/crucible` — the repo is public as of
  2026-07-06, so the old private-repo waiver is removed.
- Weave: omitted — Crucible is not a Weave-family product surface.

## Release Notes Rule

`site/changelog.html` is user-facing. Write entries as product outcomes, not
commit logs. Each entry needs a date, a version or release label, and one or two
plain-language bullets.
