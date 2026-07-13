#!/bin/bash
set -euo pipefail

mkdir -p /logs/verifier
if [ "$(cat /app/marker.txt 2>/dev/null || true)" = "crucible-harbor-ok" ]; then
  printf '1\n' > /logs/verifier/reward.txt
  printf 'marker matched\n'
else
  printf '0\n' > /logs/verifier/reward.txt
  printf 'marker missing or incorrect\n'
fi
