.PHONY: fmt vet test lint check build install-hooks

fmt:
	gofmt -w .

vet:
	go vet ./...

test:
	go test ./...

lint:
	@command -v golangci-lint >/dev/null 2>&1 && golangci-lint run ./... || echo "golangci-lint not installed, skipping"

check: fmt vet test

build:
	go build ./...

install-hooks:
	mkdir -p $$(git rev-parse --git-path hooks)
	cp scripts/pre-commit $$(git rev-parse --git-path hooks)/pre-commit
	chmod +x $$(git rev-parse --git-path hooks)/pre-commit
	@echo "Pre-commit hook installed."
