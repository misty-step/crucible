# Crucible Architecture

## Overview

Crucible uses a **council-synthesizer** pattern to transform ideas into prioritized GitHub issues.

## Components

### 1. Input Sources

Crucible reads from multiple sources:
- Repository context (issues, PRs, recent commits)
- `VISION.md` in repo root
- Strategy documents
- Human ad-hoc input (via CLI prompts)

### 2. Council Pattern

The council spawns N agents with different models/perspectives:

| Agent | Role |
|-------|------|
| Product | Focuses on user value, market fit |
| Engineering | Focuses on feasibility, technical debt |
| Design | Focuses on UX, accessibility |
| Business | Focuses on ROI, timeline |

Each agent receives the same input and provides:
- Prioritized list of items
- Rationale for each priority
- Risk assessment

### 3. Synthesizer

The synthesizer is a maximum-intelligence model that:
1. Receives all council outputs
2. Evaluates each item against VISION.md
3. Resolves conflicts between perspectives
4. Produces a unified, prioritized backlog

### 4. Issue Creator

Takes synthesizer output and creates GitHub issues with:
- Title and description
- Priority labels (p0, p1, p2, p3)
- Category labels (feature, bug, task, refactor, research)
- Milestone assignment
- Now/Next/Later quadrant

## CLI Invocation

```bash
# Run at repo root
crucible

# With specific vision doc
crucible --vision path/to/vision.md

# Interactive mode
crucible --interactive
```

## Data Flow

```
┌──────────────┐    ┌─────────┐    ┌──────────────┐    ┌─────────────┐
│ Input Sources│───▶│ Council │───▶│ Synthesizer  │───▶│ Issue Creator│
└──────────────┘    └─────────┘    └──────────────┘    └─────────────┘
```

## Extensibility

- Add new council roles by implementing the Agent interface
- Swap synthesizer model by configuring `SYNTHESIZER_MODEL`
- Add new output formats (not just GitHub issues)

## Dependencies

- Go 1.24+
- GitHub API access (via gh CLI or personal access token)
- Vision document (VISION.md)
