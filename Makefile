.PHONY: check

# Repo gate: fmt + clippy + test + build across the workspace.
# Delegates to scripts/check.sh so the gate is identical from make, CI, or a shell.
check:
	./scripts/check.sh
