package council

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
	"github.com/misty-step/crucible/internal/models"
)

func validCouncilJSON(councilor, perspective string) []byte {
	out := domain.CouncilOutput{
		Councilor:   councilor,
		Perspective: perspective,
		Confidence:  0.8,
		Summary:     "Test summary",
		Items: []domain.CouncilItem{
			{
				Title:    "Test item",
				Priority: domain.P1,
				Type:     domain.Feature,
				Effort:   domain.Medium,
			},
		},
		Meta: domain.CouncilMeta{
			ItemsProposed:  1,
			ContextQuality: domain.HighQuality,
		},
	}
	data, _ := json.Marshal(out)
	return data
}

func TestRunCouncilAllSucceed(t *testing.T) {
	t.Parallel()

	mock := cruxexec.NewMockRunner()
	// opencode is called for each perspective
	mock.Results["opencode"] = &cruxexec.RunResult{
		Stdout:   validCouncilJSON("TEST", "test"),
		ExitCode: 0,
	}

	registry := models.DefaultRegistry()
	spawner := &Spawner{
		Runner:   mock,
		Registry: registry,
		Env:      []string{"HOME=/tmp", "PATH=/usr/bin"},
	}

	input := domain.CouncilInput{
		Vision: "Test vision",
		Date:   "2026-02-14",
	}

	results, err := spawner.RunCouncil(context.Background(), input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	succeeded := 0
	for _, r := range results {
		if r.Output != nil {
			succeeded++
		}
	}
	if succeeded != 4 {
		t.Fatalf("got %d successes, want 4", succeeded)
	}
}

func TestRunCouncilPartialSuccess(t *testing.T) {
	t.Parallel()

	// 2 perspectives succeed, 2 fail with permanent auth errors.
	// Use agent-name-based routing since perspectives run in parallel.
	failSet := map[string]bool{"product": true, "design": true}

	mock := &agentMock{
		handler: func(args []string) (*cruxexec.RunResult, error) {
			agent := extractAgentArg(args)
			if failSet[agent] {
				return &cruxexec.RunResult{
					Stderr:   []byte("unauthorized 401"),
					ExitCode: 1,
				}, nil
			}
			return &cruxexec.RunResult{
				Stdout:   validCouncilJSON("TEST", agent),
				ExitCode: 0,
			}, nil
		},
	}

	registry := models.DefaultRegistry()
	spawner := &Spawner{
		Runner:   mock,
		Registry: registry,
		Env:      []string{"HOME=/tmp"},
	}

	input := domain.CouncilInput{Vision: "Test", Date: "2026-02-14"}
	results, err := spawner.RunCouncil(context.Background(), input)
	if err != nil {
		t.Fatalf("unexpected error (should allow partial): %v", err)
	}

	succeeded := 0
	for _, r := range results {
		if r.Output != nil {
			succeeded++
		}
	}
	if succeeded < 2 {
		t.Fatalf("got %d successes, want at least 2", succeeded)
	}
}

func TestRunCouncilTotalFailure(t *testing.T) {
	t.Parallel()

	mock := cruxexec.NewMockRunner()
	// Return 401 (permanent) so it doesn't retry endlessly
	mock.Results["opencode"] = &cruxexec.RunResult{
		Stderr:   []byte("unauthorized 401"),
		ExitCode: 1,
	}

	registry := models.DefaultRegistry()
	spawner := &Spawner{
		Runner:   mock,
		Registry: registry,
		Env:      []string{"HOME=/tmp"},
	}

	input := domain.CouncilInput{Vision: "Test", Date: "2026-02-14"}
	_, err := spawner.RunCouncil(context.Background(), input)
	if err == nil {
		t.Fatal("expected error for total failure")
	}
	if !strings.Contains(err.Error(), "council failed") {
		t.Fatalf("got %q, want council failed error", err)
	}
}

func TestExtractJSON(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name  string
		input string
		want  string
	}{
		{
			"code_fence",
			"Some text\n```json\n{\"key\": \"value\"}\n```\nMore text",
			`{"key": "value"}`,
		},
		{
			"bare_json",
			"Output: {\"key\": \"value\"} done",
			`{"key": "value"}`,
		},
		{
			"nested",
			`{"outer": {"inner": true}}`,
			`{"outer": {"inner": true}}`,
		},
		{
			"braces_in_strings",
			`{"msg": "use {braces} here"}`,
			`{"msg": "use {braces} here"}`,
		},
		{
			"no_json",
			"no json here",
			"",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			got := extractJSON([]byte(tc.input))
			gotStr := ""
			if got != nil {
				gotStr = string(got)
			}
			if gotStr != tc.want {
				t.Fatalf("extractJSON = %q, want %q", gotStr, tc.want)
			}
		})
	}
}

func TestRenderPrompt(t *testing.T) {
	t.Parallel()

	input := domain.CouncilInput{
		Vision: "Build great things",
		Date:   "2026-02-14",
		RepoState: domain.RepoState{
			RecentCommits: []string{"abc feat: add thing"},
			OpenIssues:    []string{"#1 Bug"},
			OpenPRs:       []string{"#2 Add feature"},
		},
		HumanInput: "Focus on MVP",
	}

	prompt := RenderPrompt(input)

	checks := []string{
		"2026-02-14",
		"Build great things",
		"abc feat: add thing",
		"#1 Bug",
		"#2 Add feature",
		"Focus on MVP",
	}

	for _, check := range checks {
		if !strings.Contains(prompt, check) {
			t.Errorf("prompt missing %q", check)
		}
	}
}

func TestRenderPromptIssuesOnly(t *testing.T) {
	t.Parallel()

	input := domain.CouncilInput{
		Date: "2026-02-14",
		RepoState: domain.RepoState{
			OpenIssues: []string{"#5 Critical bug"},
			OpenPRs:    []string{"#6 WIP feature"},
		},
	}

	prompt := RenderPrompt(input)

	if !strings.Contains(prompt, "Repository State") {
		t.Error("expected Repository State section when only issues/PRs present")
	}
	if !strings.Contains(prompt, "#5 Critical bug") {
		t.Error("prompt missing open issue")
	}
	if !strings.Contains(prompt, "#6 WIP feature") {
		t.Error("prompt missing open PR")
	}
}

func TestCategorizeError(t *testing.T) {
	t.Parallel()

	// 401 is permanent
	err := categorizeError("product", 1, "Error: 401 Unauthorized")
	if !isPermanentError(err) {
		t.Error("expected permanent error for 401")
	}

	// 500 is transient
	err = categorizeError("product", 1, "Error: 500 Internal Server Error")
	if isPermanentError(err) {
		t.Error("expected transient error for 500")
	}
}

// agentMock routes results based on the --agent flag in args.
type agentMock struct {
	handler func(args []string) (*cruxexec.RunResult, error)
}

func (m *agentMock) Run(_ context.Context, _ string, args []string, _ cruxexec.RunOpts) (*cruxexec.RunResult, error) {
	return m.handler(args)
}

// extractAgentArg finds the value of --agent in args.
func extractAgentArg(args []string) string {
	for i, arg := range args {
		if arg == "--agent" && i+1 < len(args) {
			return args[i+1]
		}
	}
	return ""
}
