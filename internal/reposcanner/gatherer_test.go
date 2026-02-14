package reposcanner

import (
	"context"
	"strings"
	"testing"

	cruxexec "github.com/misty-step/crucible/internal/exec"
)

func TestCLIGathererWithMock(t *testing.T) {
	t.Parallel()

	mock := cruxexec.NewMockRunner()
	mock.Results["git"] = &cruxexec.RunResult{
		Stdout: []byte("abc1234 feat: add something\ndef5678 fix: broken thing\n"),
	}
	mock.Results["gh"] = &cruxexec.RunResult{
		Stdout: []byte("#1 First issue\n#2 Second issue\n"),
	}
	mock.Results["find"] = &cruxexec.RunResult{
		Stdout: []byte("./main.go\n./cmd/root.go\n"),
	}

	g := &CLIGatherer{
		Runner:     mock,
		VisionPath: "nonexistent-vision.md", // graceful fallback
	}

	rc, err := g.Gather(context.Background())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(rc.RecentCommits) != 2 {
		t.Fatalf("got %d commits, want 2", len(rc.RecentCommits))
	}
	if rc.RecentCommits[0] != "abc1234 feat: add something" {
		t.Fatalf("got commit %q", rc.RecentCommits[0])
	}

	// gh is called twice (issues + PRs), mock returns same result for "gh"
	// Both OpenIssues and OpenPRs should be populated
	if len(rc.OpenIssues) != 2 {
		t.Fatalf("got %d issues, want 2", len(rc.OpenIssues))
	}

	if rc.FileTree != "./main.go\n./cmd/root.go" {
		t.Fatalf("got file tree %q", rc.FileTree)
	}

	if rc.Vision != "" {
		t.Fatalf("expected empty vision for missing file, got %q", rc.Vision)
	}
}

func TestCLIGathererGracefulFallback(t *testing.T) {
	t.Parallel()

	// Mock returns errors for everything — should not fail
	mock := cruxexec.NewMockRunner()
	mock.Errors["git"] = &testError{msg: "not a git repo"}
	mock.Errors["gh"] = &testError{msg: "gh not authenticated"}
	mock.Errors["find"] = &testError{msg: "find failed"}

	g := &CLIGatherer{Runner: mock}

	rc, err := g.Gather(context.Background())
	if err != nil {
		t.Fatalf("expected graceful fallback, got error: %v", err)
	}

	if len(rc.RecentCommits) != 0 {
		t.Fatalf("expected no commits, got %d", len(rc.RecentCommits))
	}
	if len(rc.OpenIssues) != 0 {
		t.Fatalf("expected no issues, got %d", len(rc.OpenIssues))
	}
	if rc.FileTree != "" {
		t.Fatalf("expected empty file tree, got %q", rc.FileTree)
	}
}

func TestRepoContextRender(t *testing.T) {
	t.Parallel()

	rc := &RepoContext{
		RecentCommits: []string{"abc fix: thing", "def feat: other"},
		OpenIssues:    []string{"#1 Bug report"},
		OpenPRs:       []string{"#10 Feature PR"},
		FileTree:      "./main.go\n./cmd/root.go",
		Vision:        "Build great things.",
	}

	rendered := rc.Render()

	checks := []string{
		"## Repository Context",
		"### Recent Commits",
		"abc fix: thing",
		"### Open Issues",
		"#1 Bug report",
		"### Open PRs",
		"#10 Feature PR",
		"### File Tree",
		"./main.go",
		"### Vision",
		"Build great things.",
	}

	for _, check := range checks {
		if !strings.Contains(rendered, check) {
			t.Errorf("render missing %q", check)
		}
	}
}

func TestRepoContextRenderEmpty(t *testing.T) {
	t.Parallel()

	rc := &RepoContext{}
	rendered := rc.Render()

	if !strings.Contains(rendered, "## Repository Context") {
		t.Error("should always have header")
	}
	if strings.Contains(rendered, "### Recent Commits") {
		t.Error("should skip empty sections")
	}
}

type testError struct{ msg string }

func (e *testError) Error() string { return e.msg }
