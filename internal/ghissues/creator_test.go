package ghissues

import (
	"context"
	"fmt"
	"strings"
	"sync"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
)

// seqMock returns results in order for sequential gh calls.
type seqMock struct {
	mu      sync.Mutex
	results []*cruxexec.RunResult
	errors  []error
	calls   []cruxexec.MockCall
	idx     int
}

func (m *seqMock) Run(_ context.Context, name string, args []string, opts cruxexec.RunOpts) (*cruxexec.RunResult, error) {
	m.mu.Lock()
	defer m.mu.Unlock()

	m.calls = append(m.calls, cruxexec.MockCall{Name: name, Args: args, Opts: opts})

	i := m.idx
	m.idx++

	if i < len(m.errors) && m.errors[i] != nil {
		return nil, m.errors[i]
	}
	if i < len(m.results) {
		return m.results[i], nil
	}
	return nil, fmt.Errorf("seqMock: no result for call %d", i)
}

func testItems() []domain.SynthesisItem {
	return []domain.SynthesisItem{
		{
			Title:    "Implement auth module",
			Priority: domain.P0,
			Type:     domain.Feature,
			Horizon:  domain.Now,
			Effort:   domain.Large,
			Body:     "## Problem\n\nNo auth.\n\n## Impact\n\nBlocks launch.",
			Labels:   []string{"domain/auth"},
			CouncilSupport: domain.CouncilSupport{
				ProposedBy: []string{"STRATEGIST", "ARCHITECT"},
				Consensus:  domain.Strong,
			},
			VisionAlignment: "Core to MVP",
		},
		{
			Title:    "Add rate limiting",
			Priority: domain.P2,
			Type:     domain.Task,
			Horizon:  domain.Later,
			Effort:   domain.Medium,
			Body:     "## Problem\n\nNo rate limiting.",
			Labels:   []string{"domain/api"},
		},
	}
}

func TestCreateSuccess(t *testing.T) {
	t.Parallel()

	mock := &seqMock{
		results: []*cruxexec.RunResult{
			{Stdout: []byte("https://github.com/org/repo/issues/42\n"), ExitCode: 0},
			{Stdout: []byte("https://github.com/org/repo/issues/43\n"), ExitCode: 0},
		},
	}

	c := &Creator{
		Runner:     mock,
		Milestones: DefaultMilestones(),
	}

	items := testItems()
	created, err := c.Create(context.Background(), items)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(created) != 2 {
		t.Fatalf("got %d created, want 2", len(created))
	}

	if created[0].Number != 42 {
		t.Errorf("got number %d, want 42", created[0].Number)
	}
	if created[0].Title != "Implement auth module" {
		t.Errorf("got title %q", created[0].Title)
	}
	if created[1].Number != 43 {
		t.Errorf("got number %d, want 43", created[1].Number)
	}

	// Verify gh was called with correct args
	if len(mock.calls) != 2 {
		t.Fatalf("got %d calls, want 2", len(mock.calls))
	}

	call := mock.calls[0]
	if call.Name != "gh" {
		t.Errorf("got command %q, want gh", call.Name)
	}

	args := strings.Join(call.Args, " ")
	if !strings.Contains(args, "--title Implement auth module") {
		t.Errorf("missing title in args: %s", args)
	}
	if !strings.Contains(args, "p0") {
		t.Errorf("missing p0 label in args: %s", args)
	}
	if !strings.Contains(args, "source/groom") {
		t.Errorf("missing source/groom label in args: %s", args)
	}
	if !strings.Contains(args, "domain/auth") {
		t.Errorf("missing domain/auth label in args: %s", args)
	}
	if !strings.Contains(args, "--milestone") {
		t.Errorf("missing milestone in args: %s", args)
	}
}

func TestCreateGHError(t *testing.T) {
	t.Parallel()

	mock := &seqMock{
		results: []*cruxexec.RunResult{
			{Stderr: []byte("not authenticated"), ExitCode: 1},
		},
	}

	c := &Creator{
		Runner:     mock,
		Milestones: DefaultMilestones(),
	}

	_, err := c.Create(context.Background(), testItems()[:1])
	if err == nil {
		t.Fatal("expected error for gh exit code 1")
	}
	if !strings.Contains(err.Error(), "exited 1") {
		t.Fatalf("got %q, want exit error", err)
	}
}

func TestCreateRunnerError(t *testing.T) {
	t.Parallel()

	mock := &seqMock{
		errors: []error{fmt.Errorf("connection refused")},
	}

	c := &Creator{
		Runner:     mock,
		Milestones: DefaultMilestones(),
	}

	_, err := c.Create(context.Background(), testItems()[:1])
	if err == nil {
		t.Fatal("expected error for runner failure")
	}
	if !strings.Contains(err.Error(), "connection refused") {
		t.Fatalf("got %q, want connection error", err)
	}
}

func TestCreatePartialFailure(t *testing.T) {
	t.Parallel()

	mock := &seqMock{
		results: []*cruxexec.RunResult{
			{Stdout: []byte("https://github.com/org/repo/issues/42\n"), ExitCode: 0},
			{Stderr: []byte("rate limited"), ExitCode: 1},
		},
	}

	c := &Creator{
		Runner:     mock,
		Milestones: DefaultMilestones(),
	}

	created, err := c.Create(context.Background(), testItems())
	if err == nil {
		t.Fatal("expected error for second item failure")
	}

	// First issue should still be returned
	if len(created) != 1 {
		t.Fatalf("got %d created, want 1 (partial)", len(created))
	}
	if created[0].Number != 42 {
		t.Errorf("got number %d, want 42", created[0].Number)
	}
}

func TestCreateWithRepo(t *testing.T) {
	t.Parallel()

	mock := &seqMock{
		results: []*cruxexec.RunResult{
			{Stdout: []byte("https://github.com/other/repo/issues/1\n"), ExitCode: 0},
		},
	}

	c := &Creator{
		Runner:     mock,
		Milestones: DefaultMilestones(),
		Repo:       "other/repo",
	}

	_, err := c.Create(context.Background(), testItems()[:1])
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	args := strings.Join(mock.calls[0].Args, " ")
	if !strings.Contains(args, "--repo other/repo") {
		t.Errorf("missing --repo flag in args: %s", args)
	}
}

func TestBuildLabels(t *testing.T) {
	t.Parallel()

	c := &Creator{}
	item := domain.SynthesisItem{
		Priority: domain.P1,
		Type:     domain.Feature,
		Horizon:  domain.Next,
		Effort:   domain.Small,
		Labels:   []string{"domain/api", "domain/auth"},
	}

	labels := c.buildLabels(item)
	expected := []string{"p1", "feature", "next", "source/groom", "effort/s", "domain/api", "domain/auth"}

	if len(labels) != len(expected) {
		t.Fatalf("got %d labels, want %d: %v", len(labels), len(expected), labels)
	}

	for i, want := range expected {
		if labels[i] != want {
			t.Errorf("labels[%d] = %q, want %q", i, labels[i], want)
		}
	}
}

func TestBuildLabelsNoEffort(t *testing.T) {
	t.Parallel()

	c := &Creator{}
	item := domain.SynthesisItem{
		Priority: domain.P0,
		Type:     domain.Bug,
		Horizon:  domain.Now,
	}

	labels := c.buildLabels(item)
	for _, l := range labels {
		if strings.HasPrefix(l, "effort/") {
			t.Errorf("got effort label %q for empty effort", l)
		}
	}
}

func TestBuildBody(t *testing.T) {
	t.Parallel()

	c := &Creator{}
	item := domain.SynthesisItem{
		Body: "## Problem\n\nTest body.",
		CouncilSupport: domain.CouncilSupport{
			ProposedBy: []string{"STRATEGIST"},
			Consensus:  domain.Strong,
		},
		VisionAlignment: "Aligned with MVP",
	}

	body := c.buildBody(item)

	checks := []string{
		"## Problem",
		"Test body.",
		"Council support:",
		"STRATEGIST",
		"strong",
		"Vision alignment:",
		"Aligned with MVP",
		"Created by crucible",
	}

	for _, check := range checks {
		if !strings.Contains(body, check) {
			t.Errorf("body missing %q", check)
		}
	}
}

func TestFormatDryRun(t *testing.T) {
	t.Parallel()

	items := testItems()
	output := FormatDryRun(items, DefaultMilestones())

	checks := []string{
		"# Dry Run: 2 issues",
		"## 1. Implement auth module",
		"## 2. Add rate limiting",
		"p0",
		"feature",
		"source/groom",
		"domain/auth",
		"No auth.",
		"No rate limiting.",
	}

	for _, check := range checks {
		if !strings.Contains(output, check) {
			t.Errorf("dry run output missing %q", check)
		}
	}
}

func TestExtractIssueNumber(t *testing.T) {
	t.Parallel()

	tests := []struct {
		input string
		want  int
	}{
		{"https://github.com/org/repo/issues/42\n", 42},
		{"https://github.com/org/repo/issues/1", 1},
		{`{"number": 99, "url": "https://github.com/org/repo/issues/99"}`, 99},
		{"", 0},
		{"not a url", 0},
	}

	for _, tt := range tests {
		got := extractIssueNumber(tt.input)
		if got != tt.want {
			t.Errorf("extractIssueNumber(%q) = %d, want %d", tt.input, got, tt.want)
		}
	}
}

func TestCreateContextCanceled(t *testing.T) {
	t.Parallel()

	ctx, cancel := context.WithCancel(context.Background())
	cancel() // cancel immediately

	mock := &seqMock{
		errors: []error{ctx.Err()},
	}

	c := &Creator{
		Runner:     mock,
		Milestones: DefaultMilestones(),
	}

	_, err := c.Create(ctx, testItems())
	if err == nil {
		t.Fatal("expected error for canceled context")
	}
}
