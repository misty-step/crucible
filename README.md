# Crucible

Multi-model backlog grooming and strategic planning council — Cerberus's proactive counterpart.

## Overview

Crucible is a Go CLI designed to transform raw ideas into prioritized, actionable work through a multi-model council approach. It serves as the proactive counterpart to Cerberus (which guards code quality reactively), shaping the backlog proactively.

## Development Setup

### Prerequisites

- Go 1.21+
- golangci-lint (optional, for linting)

### Make Targets

| Target | Description |
|--------|-------------|
| `make fmt` | Format code with gofmt |
| `make vet` | Run go vet |
| `make test` | Run tests |
| `make lint` | Run golangci-lint (if available) |
| `make check` | Run fmt + vet + test |
| `make build` | Build the project |
| `make install-hooks` | Install git pre-commit hook |

### Installing Pre-commit Hook

To install the pre-commit hook:

```bash
make install-hooks
```

This will copy `scripts/pre-commit` to `.git/hooks/`.

## Usage

Run at repository root:
```bash
crucible
```

Crucible reads context, spawns a multi-model council, synthesizes priorities, and outputs prioritized GitHub issues.

## Development Setup

```bash
make install-hooks  # Install git hooks
make check          # Run all checks
```

## Architecture

- **Council Pattern**: Spawns N agents with different models/perspectives
- **Synthesizer**: Maximum-intelligence model evaluates council outputs against product vision
- **Interactive**: Takes vision/strategy docs + ad-hoc human input
- **Output**: Prioritized GitHub issues with proper labels, milestones

## Status

Early development. This is a scaffold, not a finished product.

## See Also

- [VISION.md](./VISION.md) - Vision for Crucible
- [docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md) - Detailed architecture
- [QA.md](./QA.md) - QA runbook
