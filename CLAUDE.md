# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test

```bash
go build -o crucible .    # Build binary
go test ./...             # Run all tests
go test ./cmd/...         # Run tests for a single package
go test -run TestName .   # Run a single test
./crucible --version      # Verify binary
```

## Architecture

Crucible is a Go CLI that transforms ideas into prioritized GitHub issues via a **council-synthesizer** pattern:

```
Input Sources → Council (N agents) → Synthesizer → GitHub Issues
```

**Council**: Multiple AI agents with distinct perspectives (Product, Engineering, Design, Business) evaluate the same input independently. Each returns prioritized items with rationale and risk assessment.

**Synthesizer**: A maximum-intelligence model reconciles council outputs against the repo's `VISION.md`, resolves conflicts, and produces a unified priority list.

**Issue Creator**: Converts synthesizer output into GitHub issues with priority labels (p0-p3), category labels (feature/bug/task/refactor/research), milestones, and Now/Next/Later quadrants.

### Code Layout

- `main.go` — Entry point, flag parsing, CLI dispatch
- `cmd/` — Command definitions (placeholder `Command` struct, pre-cobra)
- `docs/ARCHITECTURE.md` — Detailed architecture and data flow
- `VISION.md` — Product vision; used by the synthesizer as evaluation criteria

### Key Design Decisions

- **No dependencies yet** — `go.mod` is stdlib-only. The `cmd/Command` struct is a placeholder for eventual cobra migration.
- **Planned subcommands**: `council`, `synthesize`, `issues`
- **Requires `gh` CLI** authenticated with GitHub for issue creation.

## Status

Early scaffold. Council, synthesizer, and issue creation are defined but not implemented.
