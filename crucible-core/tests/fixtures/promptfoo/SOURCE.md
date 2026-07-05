# Fixture provenance

Real, unmodified Promptfoo example configs vendored from the upstream
`promptfoo/promptfoo` repository (Apache-2.0 / MIT-licensed OSS project) to
exercise the `crucible import promptfoo` adapter against genuine external
eval definitions instead of adapter-authored synthetic YAML.

- `getting-started-promptfooconfig.yaml` — fetched 2026-07-04 from
  `https://raw.githubusercontent.com/promptfoo/promptfoo/main/examples/getting-started/promptfooconfig.yaml`.
  A clean single-prompt, multi-provider, two-test config with only directly
  mappable assertions (`contains`, `icontains`) — the happy-path fixture.
- `simple-test-promptfooconfig.yaml` and `simple-test-prompts.txt` — fetched
  2026-07-04 from
  `https://raw.githubusercontent.com/promptfoo/promptfoo/main/examples/simple-test/`.
  A richer config exercising `file://` prompt resolution, a `---`-delimited
  multi-prompt file, `equals`/`icontains` (mappable) alongside `is-json`,
  `javascript`, `python`, `similar`, `llm-rubric`, and `$ref` assertions
  (all deliberately unmappable) — the honesty/total-accounting fixture.

No file content below this notice was altered from the upstream source.
