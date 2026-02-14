# Councilor Output Schema (shared)

```json
{
  "councilor": "MERCHANT|ARCHITECT|ARTISAN|STRATEGIST",
  "perspective": "engineering|business|design|product",
  "confidence": 0.0,
  "summary": "One-sentence summary of perspective assessment",
  "items": [
    {
      "title": "Short imperative title (max 200 chars)",
      "priority": "p0|p1|p2|p3",
      "type": "feature|bug|task|refactor|research",
      "rationale": "Why this matters from this perspective",
      "risk": "Risk if not addressed",
      "effort": "s|m|l|xl",
      "dependencies": ["titles of items this depends on"],
      "evidence": "Specific file, issue, or doc that supports this"
    }
  ],
  "meta": {
    "items_proposed": 0,
    "context_quality": "high|medium|low",
    "vision_alignment": "How well context supports perspective decisions"
  }
}
```
