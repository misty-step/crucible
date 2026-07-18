#!/bin/bash
set -euo pipefail
mkdir -p /logs/verifier

reward=0
if [[ -f /app/decision.json ]] && jq -e '
  type == "object" and
  (keys | sort) == (["card_id", "reason_codes", "rejected"] | sort) and
  .card_id == "crucible-omp-harbor-adapter" and
  .reason_codes == ["ready", "acceptance_present", "dependencies_satisfied", "highest_priority"] and
  .rejected == {
    "crucible-harness-review-sweep": "blocked_dependency",
    "crucible-no-criteria-hotfix": "missing_acceptance"
  }
' /app/decision.json >/logs/verifier/decision-check.txt 2>&1; then
  reward=1
else
  {
    echo "decision.json did not satisfy the policy oracle"
    [[ -f /app/decision.json ]] && cat /app/decision.json
  } > /logs/verifier/decision-check.txt
fi

printf '%s\n' "$reward" > /logs/verifier/reward.txt
