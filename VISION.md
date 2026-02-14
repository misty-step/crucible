# Crucible Vision

## Purpose

Transform raw ideas into prioritized, actionable work through multi-model deliberation.

## Problem

Backlog grooming is often:
- Reactive — teams wait for problems to surface
- Single-perspective — one model/agent makes decisions
- Disconnected from strategic vision

## Solution

**Crucible** is a proactive backlog shaping tool that:
1. Engages multiple AI models as a "council" with diverse perspectives
2. Synthesizes council outputs against product vision using a high-intelligence model
3. Generates prioritized GitHub issues ready for implementation

## Architecture

```
Input Sources (Vision, Strategy, Repo Context, Human Input)
         │
         ▼
    ┌─────────┐
    │ Council │ ← N agents with different models/perspectives
    └────┬────┘
         │
         ▼
┌─────────────────┐
│  Synthesizer    │ ← Max-intelligence model evaluates outputs
└────────┬────────┘
         │
         ▼
   GitHub Issues (prioritized, labeled, milestone-assigned)
```

## Key Principles

1. **Interactive**: Requires human vision, strategy, and ad-hoc input
2. **Deliberative**: Multiple perspectives before decisions
3. **Vision-grounded**: Every item evaluated against product vision
4. **Actionable output**: GitHub issues, not just recommendations

## Relationship to Cerberus

- **Cerberus**: Reactive code quality guard — catches problems in PRs
- **Crucible**: Proactive backlog shaper — shapes what gets into the backlog

Together they ensure:
- Code quality is maintained (Cerberus)
- The right work gets prioritized (Crucible)

## Status

Early scaffold. Core council and synthesizer patterns to be implemented.
