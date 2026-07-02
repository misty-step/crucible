# Expose grade/adjudicate/export as MCP tools

Priority: P2 · Status: done · Estimate: M

## Goal

`crucible/src/mcp.rs` exposes exactly 5 tools today —
`crucible_validate`, `crucible_run`, `crucible_runs_list`,
`crucible_runs_show`, `crucible_runs_compare` (lines 90-231) — all read/run
verbs. `grade`, `adjudicate`, and `export` are real CLI subcommands
(`crucible/src/main.rs`) with no MCP equivalent, so an agent lane driving
Crucible headlessly mid-loop (the constellation's actual consumer per
`VISION.md`) has to shell out to the CLI instead of calling the same MCP
surface it already uses for `validate`/`run`. Add `crucible_grade`,
`crucible_adjudicate`, and `crucible_export` as MCP tools, each a thin
wrapper over the existing `main.rs` logic (do not duplicate business logic —
extract/share the function `main.rs`'s CLI handlers already call).

## Oracle

- [x] Three new tools appear in the MCP `tools/list` response
  (`crucible/src/mcp.rs:90-231` region) with JSON schemas mirroring their CLI
  flag equivalents (`--artifact`, `--key`, `--apply`, `--labels`, `--out`,
  `--arena`, `--task`, `--base-version`, etc., per the `Grade`/`Adjudicate`/
  `Export` clap variants in `main.rs:128-186`).
- [x] Each tool's handler calls the same underlying function the CLI
  subcommand calls (no re-implementation) and returns the same stable JSON
  shape the CLI's `--json` flag produces.
- [x] At least one new test per tool follows the existing pattern
  (`mcp_exposes_run_as_an_agent_intent`-style, or the `--json` round-trip
  test already used for `crucible_validate` in `crucible/tests/cli.rs`)
  invoking the tool over a real stdio JSON-RPC exchange against a fixture
  artifact/key.
- [x] `cargo test --all` and `cargo clippy --all-targets -- -D warnings` pass.

## Notes

Live-code-verified 2026-07-01: `crucible/src/mcp.rs:231-235` dispatches
exactly the 5 tools named above; `grep -n 'grade\|adjudicate\|export'
crucible/src/mcp.rs` returns no tool-handler hits (only an unrelated
`RunEval` variant string, "harbor-export-acceptance"). The 2026-07-01 groom
teardown (`~/.factory-lanes/groom/crucible.md`, section 8) flagged this
exact gap: "grade/adjudicate/export — the verbs an agent lane actually needs
mid-loop — aren't exposed" over MCP. `backlog.d/014-agent-first-surfaces-and-
honest-specs.md`'s child 4 explicitly scoped "create/update" tools out (specs
are files, not a store) but never addressed grade/adjudicate/export, which
are existing, well-defined, already-CLI-stable operations — no new schema
surface, no taste call, purely wiring the existing 3-tool pattern.

**Why:** matches OVERNIGHT.md's "spec-authoring ergonomics" / agent-first
surface bucket and a groom-report finding that is still live in the current
tree; low risk because it reuses the exact tool-registration pattern already
proven 5 times in this file tonight.

**Progress 2026-07-02 (overnight):** landed. `main.rs`'s three CLI handlers
(`run_grade`, `run_adjudicate`, `run_export`) were each split into a thin
print-only wrapper plus a shared, non-printing computation function
(`build_grade_report`, `build_judgment_queue`, `build_export`) — MCP cannot
reuse a function that `println!`s (stdout is the JSON-RPC protocol channel),
so the split was structurally required, not optional, to satisfy "no
re-implementation." `export` gained a stable `ExportReport` struct
(`crucible.export_report.v1`) — it previously had no `--json`/structured
output at all, only human `println!`s — assembled once by `build_export` and
consumed by both the CLI's print path and the new MCP tool. `print_grade_summary`
was simplified to take `&GradeReport` directly instead of five separate
scalar/struct params, since every field it needs was already on the report.
All three refactors are behavior-preserving: the full existing test suite
(including the untouched golden `crucible/tests/export.rs` round-trip
tests) passes unchanged, and CLI text/JSON output was live-verified via the
real binary before and after.

New MCP tools `crucible_grade`, `crucible_adjudicate`, `crucible_export`
registered in `mcp.rs` with JSON schemas mirroring their CLI flags. One new
end-to-end test drives all three over a single real stdio JSON-RPC session,
chaining `adjudicate`'s queue into `export`'s `--labels` the way an agent
lane actually would, and asserts `export` actually wrote `adjudications.md`
to disk (not just a structured report). `mcp_exposes_run_as_an_agent_intent`'s
exhaustive tool-name-order assertion updated for the 8-tool list.
