#!/bin/bash
set -euo pipefail
cat > /app/decision.json <<'JSON'
{
  "card_id": "crucible-omp-harbor-adapter",
  "reason_codes": [
    "ready",
    "acceptance_present",
    "dependencies_satisfied",
    "highest_priority"
  ],
  "rejected": {
    "crucible-harness-review-sweep": "blocked_dependency",
    "crucible-no-criteria-hotfix": "missing_acceptance"
  }
}
JSON
