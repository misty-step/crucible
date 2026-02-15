# QA Runbook — Crucible

## Building

```bash
go build -o crucible .
```

## Testing

```bash
go test ./...
```

## Manual QA Checklist

### Prerequisites
- [ ] Go 1.24+ installed
- [ ] `gh` CLI authenticated with GitHub
- [ ] Test repository created for validation

### Build Verification
- [ ] `go build -o crucible .` completes without errors
- [ ] Binary executes: `./crucible --version` prints version
- [ ] Binary executes: `./crucible --help` prints help

### Functional Verification
- [ ] Run crucible in test repo: `crucible`
- [ ] Verify it reads local context (VISION.md if present)
- [ ] Verify CLI prompts for human input (if interactive mode)
- [ ] Verify council subsystem placeholder works
- [ ] Verify synthesizer subsystem placeholder works
- [ ] Verify issue creation produces valid GitHub issue (if implemented)

### Acceptance Criteria Template

| Criterion | Status | Notes |
|-----------|--------|-------|
| Binary builds without errors | ☐ | |
| `--version` flag works | ☐ | |
| `--help` flag works | ☐ | |
| Reads VISION.md from repo root | ☐ | |
| Spawns multi-model council (placeholder) | ☐ | |
| Runs synthesizer (placeholder) | ☐ | |
| Creates GitHub issues with labels | ☐ | |
| Handles missing input gracefully | ☐ | |
| Configurable perspectives via --config flag | ☐ | |
| Configurable perspectives via .crucible.yml | ☐ | |

## Debugging

```bash
# Verbose output
crucible --verbose

# Dry run (don't create issues)
crucible --dry-run
```

## Known Limitations

- This is an early scaffold; core functionality is placeholder
- Council and synthesizer patterns defined but not implemented
- Issue creation requires full implementation
