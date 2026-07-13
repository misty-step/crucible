#!/usr/bin/env bash
# Crucible secret-leak scan — the security floor of the repo gate.
#
# Why this exists: an eval run invokes models with real API keys and stores
# their outputs over real PR diffs that can embed proprietary code. Run records
# therefore live *outside* version control (see `.gitignore`: the whole `runs/`
# tree is ignored). This scan is the credential backstop for everything that *is*
# tracked — fixtures, reports, configs, scripts: it fails the gate when a tracked
# file matches a known *credential shape*. Keeping raw model outputs/diffs out of
# the tree is a separate *content* policy (below), enforced by review and the
# `runs/`-is-gitignored + fixture-allowlist conventions — not by these patterns,
# which match credentials, not arbitrary proprietary text.
#
# Two layers, both gating:
#   1. grep floor (always, zero dependencies) — a small set of high-signal
#      credential patterns over git-tracked files. Deterministic and identical
#      on every machine and in CI, so the gate's guarantee never depends on a
#      tool that may be absent. gitleaks is excellent but ruleset-opaque and
#      version-dependent (in testing it silently missed a literal AKIA key), so
#      it cannot be the *only* line of defense.
#   2. gitleaks (when on PATH) — its broad ruleset (Stripe/GCP/npm/... families
#      the floor does not enumerate), run in TWO scopes, both gating:
#        * `git` mode over committed history.
#        * `dir` mode over the *explicit* tracked-file list, so an uncommitted
#          secret is caught before it lands. We pass the file list, never
#          `dir .`: gitleaks ignores `.gitignore` and would otherwise walk the
#          400 MB+ `target/` build tree.
#
# Engine selection: CRUCIBLE_LEAK_SCAN = auto (default) | grep | gitleaks.
#   auto     — grep floor + gitleaks if present (warns on stderr when absent).
#   grep     — grep floor only (CI/offline determinism; exercises the fallback).
#   gitleaks — gitleaks only (falls back to grep with a warning if absent).
#
# Scope: git-tracked files, read at their working-tree content, so a secret
# added to a tracked file fails the gate before it is ever committed.
#
# Content policy (enforced by review, not these patterns): raw model outputs and
# raw diffs are allowed *only* under the allowlisted fixture dirs
# (`crucible*/tests/fixtures/`), where they are committed deliberately as test
# inputs and must contain no live credentials. Everything else stays redacted or
# out of the tree entirely.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

mode="${CRUCIBLE_LEAK_SCAN:-auto}"

# High-signal credential families: "label|POSIX-ERE". Kept deliberately narrow
# so committed Cerberus review *prose* (which legitimately vendors words like
# "token", "secret", "external_research") never trips them — verified
# false-positive-free across the tracked tree and the fixture corpus.
patterns=(
  'private-key|-----BEGIN ([A-Z0-9]+ )*PRIVATE KEY( BLOCK)?-----'
  'aws-akia|(AKIA|ASIA)[0-9A-Z]{16}'
  'generic-assignment|(api|secret|token)[A-Za-z0-9_]{0,40}[[:space:]]*[:=][[:space:]]*["'"'"']?[A-Za-z0-9+/_=.-]{16,}'
  'bearer-token|[Bb]earer[[:space:]]+[A-Za-z0-9._-]{20,}'
  'openai-anthropic|sk-(ant-)?[A-Za-z0-9_-]{20,}'
  'stripe-key|(sk|rk)_live_[0-9A-Za-z]{24,}'
  'google-api-key|AIza[0-9A-Za-z_-]{35}'
  'jwt|eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}'
  'slack-token|xox[baprs]-[A-Za-z0-9-]{10,}'
  'github-pat|gh[pousr]_[A-Za-z0-9]{36}'
  'github-fine-pat|github_pat_[A-Za-z0-9_]{30,}'
  'url-credentials|[a-zA-Z][a-zA-Z0-9+.-]*://[^/:@[:space:]]+:[^/@[:space:]]+@'
)

known_non_secret_hit() {
  local label="$1"
  local line="$2"
  case "$label" in
    generic-assignment)
      # Shell parameter expansion defaults such as
      # `${BASTION_RUNTIME_SECRET_DIR:-/var/run/bastion-secrets}` contain
      # "secret" and a colon, but the fallback is a path, not a credential.
      [[ "$line" == *'${'*':-/'*'}'* ]] && return 0
      ;;
    url-credentials)
      # `https://x-access-token:${GITHUB_TOKEN}@...` is a runtime env reference,
      # not a committed credential value. Literal URL passwords still match.
      [[ "$line" == *':${'*'}@'* ]] && return 0
      ;;
  esac
  return 1
}

# Tracked paths, read NUL-delimited so unusual paths survive. bash 3.2 (macOS's
# system shell) has no `mapfile`, so populate the array with a portable read
# loop — the floor's zero-dependency guarantee must hold on the oldest shell we
# target, and an empty array here would silently pass the gate.
tracked_files=()
while IFS= read -r -d '' f; do
  # A pending deletion is still present in the index but has no working-tree
  # content to scan. Skip it; committed-history mode below still covers the
  # last committed bytes.
  [[ -f "$f" ]] || continue
  tracked_files+=("$f")
done < <(git ls-files -z)

# grep floor. Prints `path:line` (NEVER the matched secret) per hit so the gate
# log itself never leaks. Returns 1 if anything matched, 0 if clean.
grep_floor() {
  ((${#tracked_files[@]})) || return 0
  local entry label pat rc=0 hit
  for entry in "${patterns[@]}"; do
    label="${entry%%|*}"
    pat="${entry#*|}"
    # Feed the file list through `xargs -0` so a large tree can never overflow
    # ARG_MAX and make the scan silently exec-fail; `-H` forces the path prefix
    # even on a one-file batch. A real hit emits a line, which sets rc=1.
    while IFS= read -r hit; do
      content="${hit#*:}"
      content="${content#*:}"
      known_non_secret_hit "$label" "$content" && continue
      rc=1
      # Keep only `path:line`; drop the matched content to avoid echoing secrets.
      printf '  LEAK[%s] %s\n' "$label" "$(awk -F: '{print $1 ":" $2}' <<<"$hit")"
    done < <(printf '%s\0' "${tracked_files[@]}" | xargs -0 grep -nHIiE -e "$pat" 2>/dev/null || true)
  done
  return "$rc"
}

# gitleaks broad pass. Two scopes, both gating: `git` mode over committed
# history, then `dir` mode over the explicit tracked-file list (working-tree
# content, so an uncommitted secret is caught). --redact keeps captured output
# secret-free. Returns 1 on findings, 0 if clean. Caller guarantees gitleaks is
# on PATH.
gitleaks_pass() {
  local out rc=0 grc=0 drc=0
  out="$(gitleaks git . --no-banner --redact --exit-code 1 --log-level error 2>&1)" || grc=$?
  if ((grc != 0)); then
    echo "  gitleaks findings — committed history (redacted):"
    [ -n "$out" ] && printf '%s\n' "$out" | sed 's/^/    /'
    rc=1
  fi
  # `gitleaks dir` takes exactly one positional [path] (its own usage says so,
  # and the `file`/`directory` aliases confirm it); handing it the whole
  # tracked-file list as separate argv words does not error loudly — past some
  # combined-length threshold it silently joins them into one bogus path and
  # every subsequent gate run passes trivially (a false "clean") until, at
  # ~95 tracked files here, the joined string tripped the OS's own
  # ENAMETOOLONG and the gate started failing on target/ noise instead. So one
  # file at a time, not one call for every file.
  local f findings=0
  for f in "${tracked_files[@]}"; do
    out="$(gitleaks dir --no-banner --redact --exit-code 1 --log-level error "$f" 2>&1)" || drc=$?
    if ((drc != 0)); then
      findings=1
      echo "  gitleaks findings — working tree (redacted):"
      [ -n "$out" ] && printf '%s\n' "$out" | sed 's/^/    /'
      drc=0
    fi
  done
  ((findings)) && rc=1
  return "$rc"
}

have_gitleaks=0
command -v gitleaks >/dev/null 2>&1 && have_gitleaks=1

fail=0
coverage=""

case "$mode" in
  grep)
    grep_floor || fail=1
    coverage="grep: grep floor only"
    ;;
  gitleaks)
    if ((have_gitleaks)); then
      gitleaks_pass || fail=1
      coverage="gitleaks: broad ruleset only"
    else
      echo "leak-scan: CRUCIBLE_LEAK_SCAN=gitleaks but gitleaks not on PATH; broad ruleset SKIPPED — grep floor only" >&2
      grep_floor || fail=1
      coverage="gitleaks requested but absent: grep floor only"
    fi
    ;;
  auto)
    grep_floor || fail=1
    if ((have_gitleaks)); then
      gitleaks_pass || fail=1
      coverage="auto: grep floor + gitleaks broad ruleset"
    else
      echo "leak-scan: gitleaks not on PATH; broad ruleset SKIPPED — grep floor only" >&2
      coverage="auto: grep floor ONLY (gitleaks absent — broad ruleset SKIPPED)"
    fi
    ;;
  *)
    echo "leak-scan: unknown CRUCIBLE_LEAK_SCAN='$mode' (want auto|grep|gitleaks)" >&2
    exit 2
    ;;
esac

if ((fail)); then
  cat >&2 <<'MSG'
==> leak scan FAILED: a tracked file matches a secret/content-leak pattern.
    Remove the credential (rotate it if it was ever real) or, if this is an
    intentional test input, place it under an allowlisted fixture dir and ensure
    it carries no live secret. Run records with real diffs belong under runs/
    (gitignored), never committed raw.
MSG
  exit 1
fi

echo "==> leak scan clean (${coverage})"
