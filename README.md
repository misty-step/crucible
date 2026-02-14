# Crucible

Multi-model backlog grooming and strategic planning council — Cerberus's proactive counterpart.

## Overview

Crucible is a Go CLI designed to transform raw ideas into prioritized, actionable work through a multi-model council approach. It serves as the proactive counterpart to Cerberus (which guards code quality reactively), shaping the backlog proactively.

## Usage

Run at repository root:
```bash
crucible
```

Crucible reads context, spawns a multi-model council, synthesizes priorities, and outputs prioritized GitHub issues.

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
