.PHONY: fmt vet test lint check build install-hooks

# Go build variables
GO = go
GOFMT = gofmt

# Default target
all: check build

# Format code
fmt:
	$(GOFMT) -w .

# Run go vet
vet:
	$(GO) vet ./...

# Run tests
test:
	$(GO) test ./...

# Run linter (if available)
lint:
	@if command -v golangci-lint > /dev/null 2>&1; then \
		golangci-lint run; \
	else \
		echo "golangci-lint not found, skipping lint"; \
	fi

# Run all checks (fmt, vet, test)
check: fmt vet test

# Build the project
build:
	$(GO) build ./...

# Install pre-commit hook
install-hooks:
	mkdir -p $$(git rev-parse --git-path hooks)
	cp scripts/pre-commit $$(git rev-parse --git-path hooks)/pre-commit
