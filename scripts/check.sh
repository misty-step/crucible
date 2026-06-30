#!/usr/bin/env bash
# Crucible repo gate: formatting, lints, tests, and build across the workspace.
# This script is the single source of truth for "is the tree green?" — run it
# locally before pushing and wire it into CI unchanged.
set -euo pipefail

# Run from the repository root regardless of the caller's working directory so
# the gate behaves identically from a Makefile, CI, or an interactive shell.
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Security floor first: fail fast on a leaked credential or raw model
# output/diff before spending minutes on the build. Uses gitleaks when present,
# else a self-contained high-signal grep scan (see scripts/leak-scan.sh).
echo "==> scripts/leak-scan.sh"
bash "$repo_root/scripts/leak-scan.sh"

echo "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> cargo clippy --all-targets -- -D warnings"
cargo clippy --all-targets -- -D warnings

echo "==> cargo test --all"
cargo test --all

echo "==> cargo build --all"
cargo build --all

# Documentation must build warning-free: broken/ambiguous intra-doc links,
# links into private items, and redundant explicit targets all fail the gate so
# the rustdoc surface cannot silently rot.
echo "==> RUSTDOCFLAGS=\"-D warnings\" cargo doc --no-deps"
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

echo "==> gate passed"
