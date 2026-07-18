You are selecting the next claimable work card from a frozen board snapshot.

Read `/app/evidence/selection-policy.json` and `/app/evidence/cards.json`. Apply the policy exactly; do not use card ordering or outside knowledge. Write `/app/decision.json` as one JSON object with exactly these fields:

- `card_id`: selected card id
- `reason_codes`: policy reason codes in the policy's required order
- `rejected`: an object containing rejection codes for exactly the two audit cards named by `required_rejection_audits`

Use only reason and rejection codes declared by the policy. Do not change files under `/app/evidence`.
