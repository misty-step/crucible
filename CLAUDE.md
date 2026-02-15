# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test

```bash
go build -o crucible .         # Build binary
go test ./...                  # Run all tests
go test ./internal/domain/...  # Run tests for a single package
go test -run TestName ./...    # Run a single test
go vet ./...                   # Lint (also runs in CI)
./crucible --version           # Verify binary
```

## Architecture

Crucible is a Go CLI (cobra) that transforms ideas into prioritized GitHub issues via a **council-synthesizer** pattern:

```
Input Sources → Council (N agents) → Synthesizer → GitHub Issues
```

### Pipeline

1. **Council** — Multiple AI agents with distinct perspectives (Product, Engineering, Design, Business) evaluate input independently. Each returns `CouncilOutput` with prioritized items, rationale, risk, and confidence scores.

2. **Synthesizer** — A max-intelligence model reconciles council outputs against `VISION.md`, resolves conflicts, produces a unified `SynthesisResult`.

3. **Emitter** — Converts `SynthesisItem`s into GitHub issues with priority labels (p0-p3), category labels (feature/bug/task/refactor/research), effort estimates (s/m/l/xl), and Now/Next/Later horizon.

### Code Layout

- `main.go` — Entry point, calls `cmd.Execute()`
- `cmd/` — Cobra commands: `root.go` (global flags), `council.go`, `synthesize.go`, `issues.go`
- `internal/domain/` — Shared types (`types.go`), interfaces (`interfaces.go`), validation (`validate.go`)
- `internal/models/` — OpenRouter model registry with per-perspective primary/fallback chains
- `internal/exec/` — Input sanitization (`SanitizeArg`, `SanitizeTitle`) and env filtering for child processes
- `VISION.md` — Product vision; used by the synthesizer as evaluation criteria

### Key Interfaces (`internal/domain/interfaces.go`)

- `Agent` — `Evaluate(ctx, CouncilInput) → CouncilOutput`
- `Synthesizer` — `Synthesize(ctx, SynthesisInput) → SynthesisResult`
- `Emitter` — `Emit(ctx, []SynthesisItem) → []CreatedIssue`

### Model Registry (`internal/models/registry.go`)

Each council perspective has a primary model + fallback chain via OpenRouter. Synthesis uses `claude-opus-4.6` with no fallback (quality is non-negotiable). `Registry.NextModel()` walks the chain on failure.

### Security Invariants

- **Argument injection**: All model-derived strings passed to `exec.Command` must go through `exec.SanitizeArg()` (strips leading dashes, null bytes)
- **Env allowlist**: Child processes only receive `AllowedEnvKeys` via `exec.FilterEnv()` — prevents leaking secrets
- **Output size limit**: Model output reads capped at 1MB via `exec.LimitedReader()`

### Global Flags

`--verbose`, `--vision <path>` (default `VISION.md`), `--dry-run`

### Environment

- Requires `OPENROUTER_API_KEY` for model access
- Requires `gh` CLI authenticated for issue creation
- Go 1.24+ (version from `go.mod`)

### CI

GitHub Actions runs `go build`, `go vet`, `go test` on push/PR to main/master. Semantic release on `master` branch.

## Status

Cobra CLI wired, domain types and validation implemented with tests. Council, synthesizer, and emitter are stub commands awaiting implementation.
