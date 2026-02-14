package council

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"

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

// --- ExtractJSON tests ---

func TestExtractJSONWithBracesInsideStrings(t *testing.T) {
	t.Parallel()

	raw := []byte("noise before\n{\"note\":\"ignore { braces } inside\",\"nested\":{\"value\":\"{escaped}\"}}\nnoise after")
	got := ExtractJSON(raw)
	if got == nil {
		t.Fatal("expected JSON bytes")
	}

	want := "{\"note\":\"ignore { braces } inside\",\"nested\":{\"value\":\"{escaped}\"}}"
	if string(got) != want {
		t.Fatalf("got %q, want %q", string(got), want)
	}
}

func TestExtractJSONHandlesLeadingLiteralBraces(t *testing.T) {
	t.Parallel()

	raw := []byte("{{ { \"note\": \"not json\" } }}")
	got := ExtractJSON(raw)
	if got == nil {
		t.Fatal("expected JSON bytes")
	}

	var payload map[string]string
	if err := json.Unmarshal(got, &payload); err != nil {
		t.Fatalf("invalid JSON: %v", err)
	}
	if payload["note"] != "not json" {
		t.Fatalf("got note=%q, want %q", payload["note"], "not json")
	}
}

func TestExtractJSONWithFenceBackticksInString(t *testing.T) {
	t.Parallel()

	raw := []byte("```json\n{ \"note\": \"```go\\ncode\\n```\" }\n```\n")
	got := ExtractJSON(raw)
	if got == nil {
		t.Fatal("expected JSON bytes")
	}

	var payload map[string]string
	if err := json.Unmarshal(got, &payload); err != nil {
		t.Fatalf("invalid JSON: %v", err)
	}
	if payload["note"] != "```go\ncode\n```" {
		t.Fatalf("got note=%q, want %q", payload["note"], "```go\ncode\n```")
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
			got := ExtractJSON([]byte(tc.input))
			gotStr := ""
			if got != nil {
				gotStr = string(got)
			}
			if gotStr != tc.want {
				t.Fatalf("ExtractJSON = %q, want %q", gotStr, tc.want)
			}
		})
	}
}

// --- Error classification tests ---

func TestIsPermanentError(t *testing.T) {
	t.Parallel()

	wrapped := fmt.Errorf("outer: %w", &permanentError{fmt.Errorf("auth failure")})
	if !isPermanentError(wrapped) {
		t.Fatal("expected permanent error to be detected via wrapped type")
	}

	if isPermanentError(fmt.Errorf("not permanent")) {
		t.Fatal("expected non-permanent error")
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

// --- Council execution tests ---

func TestRunCouncilSpawnsPerspectivesInParallel(t *testing.T) {
	t.Parallel()

	runner := &concurrentRunner{}
	spawner := &Spawner{
		Runner:   runner,
		Registry: models.DefaultRegistry(),
	}

	_, err := spawner.RunCouncil(context.Background(), domain.CouncilInput{})
	if err != nil {
		t.Fatalf("RunCouncil() unexpected error: %v", err)
	}

	runner.mu.Lock()
	defer runner.mu.Unlock()

	expected := len(spawner.councilPerspectives())
	if runner.calls != expected {
		t.Fatalf("got %d runner calls, want %d", runner.calls, expected)
	}
	if runner.maxActive < 2 {
		t.Fatalf("expected parallel execution, maxActive=%d", runner.maxActive)
	}
}

func TestRunCouncilAllSucceed(t *testing.T) {
	t.Parallel()

	mock := cruxexec.NewMockRunner()
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

// --- Fallback and retry tests ---

func TestRunPerspectiveFallsBackToSecondaryModel(t *testing.T) {
	t.Parallel()

	runner := &scriptedRunner{
		responses: []scriptResponse{
			{
				Result: &cruxexec.RunResult{
					ExitCode: 1,
					Stderr:   []byte("401 unauthorized"),
				},
			},
			{
				Result: &cruxexec.RunResult{
					Stdout:   validCouncilJSON("TEST", "product"),
					ExitCode: 0,
				},
			},
		},
	}
	spawner := &Spawner{
		Runner:   runner,
		Registry: models.DefaultRegistry(),
	}

	result := spawner.runPerspective(context.Background(), "product", "prompt")
	if result.Output == nil {
		t.Fatal("expected output from fallback model")
	}
	if result.Model != "google/gemini-3-flash-preview" {
		t.Fatalf("got model %q, want google/gemini-3-flash-preview", result.Model)
	}
	if result.Retries != 0 {
		t.Fatalf("got retries %d, want 0", result.Retries)
	}

	runner.mu.Lock()
	defer runner.mu.Unlock()
	if len(runner.calls) != 2 {
		t.Fatalf("got %d calls, want 2", len(runner.calls))
	}
	if got := runner.calls[0].Args[4]; got != "openrouter/anthropic/claude-sonnet-4.5" {
		t.Fatalf("first model call was %q, want anthropic primary", got)
	}
	if got := runner.calls[1].Args[4]; got != "openrouter/google/gemini-3-flash-preview" {
		t.Fatalf("fallback call was %q, want gemini fallback", got)
	}
}

func TestTryModelWithRetriesHonorsRetriesBeforeCancellation(t *testing.T) {
	t.Parallel()

	runner := &scriptedRunner{
		responses: []scriptResponse{
			{
				Result: &cruxexec.RunResult{
					ExitCode: 1,
					Stderr:   []byte("temporary issue"),
				},
			},
			{
				Result: &cruxexec.RunResult{
					ExitCode: 1,
					Stderr:   []byte("temporary issue"),
				},
			},
		},
	}
	spawner := &Spawner{
		Runner:   runner,
		Registry: models.DefaultRegistry(),
	}

	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()

	result := spawner.tryModelWithRetries(ctx, "product", models.Model{ID: "moonshotai/kimi-k2.5"}, "prompt", 0)
	if result.Retries != 1 {
		t.Fatalf("got retries %d, want 1", result.Retries)
	}
	if !errors.Is(result.Error, context.DeadlineExceeded) {
		t.Fatalf("got error %v, want context deadline exceeded", result.Error)
	}

	runner.mu.Lock()
	defer runner.mu.Unlock()
	if len(runner.calls) != 1 {
		t.Fatalf("got %d calls, want 1", len(runner.calls))
	}
}

func TestRunPerspectiveStopsFallbackOnCancellation(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	runner := &scriptedRunner{
		responses: []scriptResponse{
			{
				Result: &cruxexec.RunResult{
					ExitCode: 1,
					Stderr:   []byte("temporary issue"),
				},
			},
			{
				Result: &cruxexec.RunResult{
					Stdout:   validCouncilJSON("TEST", "product"),
					ExitCode: 0,
				},
			},
		},
	}
	spawner := &Spawner{
		Runner:   runner,
		Registry: models.DefaultRegistry(),
	}

	result := spawner.runPerspective(ctx, "product", "prompt")
	if !errors.Is(result.Error, context.Canceled) {
		t.Fatalf("got error %v, want context canceled", result.Error)
	}

	runner.mu.Lock()
	defer runner.mu.Unlock()
	if len(runner.calls) != 1 {
		t.Fatalf("expected 1 call before cancellation stop, got %d", len(runner.calls))
	}
}

// --- Agent invocation tests ---

func TestInvokeAgentValidationFailure(t *testing.T) {
	t.Parallel()

	mock := cruxexec.NewMockRunner()
	mock.Results["opencode"] = &cruxexec.RunResult{
		Stdout:   []byte(`{"councilor":"STRATEGIST","confidence":0.9}`),
		ExitCode: 0,
	}

	spawner := &Spawner{
		Runner:   mock,
		Registry: models.DefaultRegistry(),
	}

	_, err := spawner.invokeAgent(context.Background(), "product", models.Model{ID: "moonshotai/kimi-k2.5"}, "prompt", 0)
	if err == nil {
		t.Fatal("expected validation error")
	}
	if !strings.Contains(err.Error(), "invalid council output") {
		t.Fatalf("got error %q, want invalid council output", err)
	}
}

func TestInvokeAgentSanitizesPrompt(t *testing.T) {
	t.Parallel()

	mock := cruxexec.NewMockRunner()
	spawner := &Spawner{
		Runner: mock,
		Env:    []string{"HOME=/tmp"},
	}

	output := map[string]interface{}{
		"councilor":   "STRATEGIST",
		"perspective": "product",
		"confidence":  0.9,
		"summary":     "ok",
	}
	stdout, _ := json.Marshal(output)

	mock.Results["opencode"] = &cruxexec.RunResult{
		Stdout:   stdout,
		ExitCode: 0,
	}

	_, err := spawner.invokeAgent(context.Background(), "product", models.Model{ID: "moonshotai/kimi-k2.5"}, " --payload\x00value ", 0)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(mock.Calls) != 1 {
		t.Fatalf("expected 1 runner call, got %d", len(mock.Calls))
	}

	args := mock.Calls[0].Args
	got := args[len(args)-1]
	want := "payloadvalue"
	if got != want {
		t.Fatalf("got sanitized prompt %q, want %q", got, want)
	}
}

// --- Render prompt tests ---

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

// --- Test helpers ---

type concurrentRunner struct {
	mu        sync.Mutex
	calls     int
	active    int
	maxActive int
}

func (r *concurrentRunner) Run(_ context.Context, _ string, _ []string, _ cruxexec.RunOpts) (*cruxexec.RunResult, error) {
	r.mu.Lock()
	r.calls++
	r.active++
	if r.active > r.maxActive {
		r.maxActive = r.active
	}
	r.mu.Unlock()

	time.Sleep(40 * time.Millisecond)

	r.mu.Lock()
	r.active--
	r.mu.Unlock()

	return &cruxexec.RunResult{
		Stdout:   validCouncilJSON("TEST", "test"),
		ExitCode: 0,
	}, nil
}

type scriptedRunner struct {
	mu        sync.Mutex
	calls     []scriptedCall
	responses []scriptResponse
}

type scriptedCall struct {
	Args []string
}

type scriptResponse struct {
	Result *cruxexec.RunResult
	Error  error
}

func (r *scriptedRunner) Run(_ context.Context, _ string, args []string, _ cruxexec.RunOpts) (*cruxexec.RunResult, error) {
	r.mu.Lock()
	idx := len(r.calls)
	r.calls = append(r.calls, scriptedCall{Args: append([]string{}, args...)})
	r.mu.Unlock()

	if idx >= len(r.responses) {
		return nil, fmt.Errorf("unexpected call %d", idx)
	}

	resp := r.responses[idx]
	return resp.Result, resp.Error
}

// agentMock routes results based on the --agent flag in args.
type agentMock struct {
	handler func(args []string) (*cruxexec.RunResult, error)
}

func (m *agentMock) Run(_ context.Context, _ string, args []string, _ cruxexec.RunOpts) (*cruxexec.RunResult, error) {
	return m.handler(args)
}

func extractAgentArg(args []string) string {
	for i, arg := range args {
		if arg == "--agent" && i+1 < len(args) {
			return args[i+1]
		}
	}
	return ""
}
