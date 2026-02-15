package council

import (
	"strings"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
)

func TestRenderPromptIncludesAllSections(t *testing.T) {
	t.Parallel()

	input := domain.CouncilInput{
		Date:   "2026-02-14",
		Vision: "Ship a polished CLI experience",
		RepoState: domain.RepoState{
			RecentCommits: []string{"feat: add synthesizer"},
			OpenIssues:    []string{"#100 bug in registry"},
			OpenPRs:       []string{"#99 improve docs"},
			FileTree:      "cmd/\ninternal/\n",
		},
		HumanInput: "Prioritize reliability over new features",
	}

	got := RenderPrompt(input)

	requireContains := []string{
		"Date: 2026-02-14",
		"## Vision",
		"## Repository State",
		"### Recent Commits",
		"- feat: add synthesizer",
		"### Open Issues",
		"- #100 bug in registry",
		"### Open PRs",
		"- #99 improve docs",
		"### File Tree",
		"cmd/",
		"## Human Input",
		"## Task",
	}

	for _, check := range requireContains {
		if !strings.Contains(got, check) {
			t.Fatalf("rendered prompt missing %q", check)
		}
	}
}
