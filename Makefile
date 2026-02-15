.PHONY: fmt vet test lint check build install-hooks

fmt:
	@echo "==> gofmt"
	@gofmt -w .
	@if command -v goimports >/dev/null 2>&1; then \
		echo "==> goimports"; \
		goimports -w .; \
	else \
		echo "==> goimports not found; skipping"; \
	fi

vet:
	@echo "==> go vet"
	@go vet ./...

test:
	@echo "==> go test"
	@go test ./...

lint:
	@if command -v golangci-lint >/dev/null 2>&1; then \
		echo "==> golangci-lint"; \
		golangci-lint run ./...; \
	else \
		echo "==> golangci-lint not found; skipping"; \
	fi

check: fmt vet test

build:
	@echo "==> go build"
	@go build ./...

install-hooks:
	@HOOKS_DIR="$$(git rev-parse --git-path hooks 2>/dev/null)"; \
	if [ -z "$$HOOKS_DIR" ]; then \
		echo "error: not a git repository (cannot install hooks)"; \
		exit 1; \
	fi; \
	mkdir -p "$$HOOKS_DIR"; \
	cp scripts/pre-commit "$$HOOKS_DIR/pre-commit"; \
	chmod +x "$$HOOKS_DIR/pre-commit"; \
	echo "Installed pre-commit hook to $$HOOKS_DIR/pre-commit"
