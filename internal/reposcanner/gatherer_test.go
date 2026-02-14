package reposcanner

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	domain "github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
)

func TestCLIGathererWithMock(t *testing.T) {
	t.Parallel()

	tmp := t.TempDir()
	if err := os.WriteFile(filepath.Join(tmp, "main.go"), []byte("package main\n"), 0o644); err != nil {
		t.Fatalf("write file: %v", err)
	}
	if err := os.MkdirAll(filepath.Join(tmp, "cmd"), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(tmp, "cmd", "root.go"), []byte("package cmd\n"), 0o644); err != nil {
		t.Fatalf("write file: %v", err)
	}

	mock := cruxexec.NewMockRunner()
	mock.Results["git"] = &cruxexec.RunResult{
		Stdout: []byte("abc1234 feat: add something\ndef5678 fix: broken thing\n"),
	}
	mock.Results[mockCommandKey("gh", []string{"issue", "list", "--state", "open", "--limit", "30", "--json", "number,title", "--jq", `.[] | "#\(.number) \(.title)"`})] = &cruxexec.RunResult{
		Stdout: []byte("#1 First issue\n#2 Second issue\n"),
	}
	mock.Results[mockCommandKey("gh", []string{"pr", "list", "--state", "open", "--limit", "20", "--json", "number,title", "--jq", `.[] | "#\(.number) \(.title)"`})] = &cruxexec.RunResult{
		Stdout: []byte("#10 New PR\n#11 Improve docs\n"),
	}

	g := &CLIGatherer{
		Runner:     mock,
		VisionPath: "nonexistent-vision.md", // graceful fallback
		Dir:        tmp,
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

	// gh is called twice (issues + PRs). Mock is configured per command+args.
	// Both OpenIssues and OpenPRs should be populated.
	if len(rc.OpenIssues) != 2 {
		t.Fatalf("got %d issues, want 2", len(rc.OpenIssues))
	}
	if len(rc.OpenPRs) != 2 {
		t.Fatalf("got %d PRs, want 2", len(rc.OpenPRs))
	}
	if len(mock.Calls) != 3 {
		t.Fatalf("got %d command calls, want 3", len(mock.Calls))
	}
	if err := assertCalledWithArgs(mock.Calls, "gh", []string{"issue", "list", "--state", "open", "--limit", "30", "--json", "number,title", "--jq", `.[] | "#\(.number) \(.title)"`}); err != nil {
		t.Fatalf("expected issue call: %v", err)
	}
	if err := assertCalledWithArgs(mock.Calls, "gh", []string{"pr", "list", "--state", "open", "--limit", "20", "--json", "number,title", "--jq", `.[] | "#\(.number) \(.title)"`}); err != nil {
		t.Fatalf("expected PR call: %v", err)
	}

	if rc.FileTree != "./cmd/root.go\n./main.go" {
		t.Fatalf("got file tree %q", rc.FileTree)
	}

	if rc.Vision != "" {
		t.Fatalf("expected empty vision for missing file, got %q", rc.Vision)
	}
}

func mockCommandKey(name string, args []string) string {
	if len(args) == 0 {
		return name
	}
	return name + "\x1f" + strings.Join(args, "\x1e")
}

func TestCLIGathererGracefulFallback(t *testing.T) {
	t.Parallel()

	tmp := t.TempDir()

	// Mock returns errors for everything — should not fail
	mock := cruxexec.NewMockRunner()
	mock.Errors["git"] = &testError{msg: "not a git repo"}
	mock.Errors["gh"] = &testError{msg: "gh not authenticated"}

	g := &CLIGatherer{Runner: mock, Dir: tmp}

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
	if len(rc.OpenPRs) != 0 {
		t.Fatalf("expected no PRs, got %d", len(rc.OpenPRs))
	}
	if rc.FileTree != "" {
		t.Fatalf("expected empty file tree, got %q", rc.FileTree)
	}
}

func TestCLIGathererPartialFailure(t *testing.T) {
	t.Parallel()

	tmp := t.TempDir()
	if err := os.WriteFile(filepath.Join(tmp, "main.go"), []byte("package main\n"), 0o644); err != nil {
		t.Fatalf("write file: %v", err)
	}

	mock := cruxexec.NewMockRunner()
	mock.Results["git"] = &cruxexec.RunResult{
		Stdout: []byte("abc1234 feat: add something\n"),
	}
	mock.Errors["gh"] = &testError{msg: "gh not authenticated"}

	g := &CLIGatherer{
		Runner: mock,
		Dir:    tmp,
	}

	rc, err := g.Gather(context.Background())
	if err != nil {
		t.Fatalf("expected graceful fallback, got error: %v", err)
	}
	if len(rc.RecentCommits) != 1 {
		t.Fatalf("expected 1 commit, got %d", len(rc.RecentCommits))
	}
	if len(rc.OpenIssues) != 0 {
		t.Fatalf("expected no issues, got %d", len(rc.OpenIssues))
	}
	if len(rc.OpenPRs) != 0 {
		t.Fatalf("expected no PRs, got %d", len(rc.OpenPRs))
	}
	if rc.FileTree != "./main.go" {
		t.Fatalf("got file tree %q", rc.FileTree)
	}
}

func TestCLIGathererReadsVisionFromDir(t *testing.T) {
	t.Parallel()

	tmp := t.TempDir()
	if err := os.WriteFile(filepath.Join(tmp, "VISION.md"), []byte("Build great things.\n"), 0o644); err != nil {
		t.Fatalf("write vision file: %v", err)
	}

	mock := cruxexec.NewMockRunner()
	g := &CLIGatherer{
		Runner: mock,
		Dir:    tmp,
	}

	rc, err := g.Gather(context.Background())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(rc.Vision, "Build great things.") {
		t.Fatalf("expected vision content, got %q", rc.Vision)
	}
}

func TestCLIGathererWithNilRunner(t *testing.T) {
	t.Parallel()

	tmp := t.TempDir()

	// No Runner configured should fallback to OSRunner.
	rc, err := (&CLIGatherer{
		Dir: tmp,
	}).Gather(context.Background())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if rc == nil {
		t.Fatal("expected gather result")
	}
}

func assertCalledWithArgs(calls []cruxexec.MockCall, name string, args []string) error {
	for _, call := range calls {
		if call.Name != name {
			continue
		}
		if len(call.Args) == len(args) && strings.Join(call.Args, "\x00") == strings.Join(args, "\x00") {
			return nil
		}
	}
	return fmt.Errorf("%s not called with args %q", name, args)
}

func TestRepoContextRender(t *testing.T) {
	t.Parallel()

	rc := &RepoContext{
		RepoState: domain.RepoState{
			RecentCommits: []string{"abc fix: thing", "def feat: other"},
			OpenIssues:    []string{"#1 Bug report"},
			OpenPRs:       []string{"#10 Feature PR"},
			FileTree:      "./main.go\n./cmd/root.go",
		},
		Vision: "Build great things.",
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
