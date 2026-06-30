# Answer-key adjudications — pr-review-v0

The standing workflow for "the candidate reported a finding the answer key does
not list" (DESIGN.md, Adjudication): each disputed finding is adjudicated here,
then either **ACCEPT** — extend the key and oracle solution, bump the arena
version (prior cross-version averaging becomes invalid; baselines re-run before
any new comparison) — or **OUT-OF-SCOPE** — record the rationale and leave the
key unchanged. Keys improve instead of silently punishing reviewers better than
their author.

| id | date | task | finding | ruling |
|---|---|---|---|---|
| ADJ-1 | 2026-06-29 | py-file-cache | set() uses a deterministic temp file per key, so two concurrent wri… | **ACCEPT** → key extended, arena 0.2.0 → 0.3.0 |
| ADJ-2 | 2026-06-29 | py-file-cache | os.rename(tmp, _path(key)) raises FileExistsError on Windows when t… | **OUT-OF-SCOPE** |
| ADJ-3 | 2026-06-29 | py-file-cache | Prefer an f-string over string concatenation here. | **OUT-OF-SCOPE** |

## ADJ-1 — concurrency at cache.py:23 (ACCEPT)

- **Date:** 2026-06-29
- **Task:** py-file-cache
- **Finding id:** F3
- **Location:** cache.py:23
- **Category:** concurrency
- **Severity:** blocking
- **Verdict:** keep
- **Disposition:** in-scope
- **Ruling:** ACCEPT
- **Version:** 0.2.0 → 0.3.0
- **Claim:** set() uses a deterministic temp file per key, so two concurrent writers interleave writes to the same .tmp and the rename can publish a corrupted partial payload.\n\nThe atomic write-then-rename pattern needs a unique temp file per writer.
- **Conditions:** latency_ms=90000 saw_grader_before_commit=false timestamp=2026-06-29T18:12:00Z

## ADJ-2 — portability at cache.py:26 (OUT-OF-SCOPE)

- **Date:** 2026-06-29
- **Task:** py-file-cache
- **Finding id:** F1
- **Location:** cache.py:26
- **Category:** portability
- **Severity:** minor
- **Verdict:** keep
- **Disposition:** out-of-contract
- **Ruling:** OUT-OF-SCOPE
- **Claim:** os.rename(tmp, _path(key)) raises FileExistsError on Windows when the destination exists; os.replace is the portable atomic move.
- **Conditions:** latency_ms=45000 saw_grader_before_commit=false timestamp=2026-06-29T18:15:00Z

## ADJ-3 — style at app.py:5 (OUT-OF-SCOPE)

- **Date:** 2026-06-29
- **Task:** py-file-cache
- **Finding id:** F2
- **Location:** app.py:5
- **Category:** style
- **Severity:** minor
- **Verdict:** noise
- **Disposition:** in-scope
- **Ruling:** OUT-OF-SCOPE
- **Claim:** Prefer an f-string over string concatenation here.
- **Conditions:** latency_ms=30000 saw_grader_before_commit=false timestamp=
