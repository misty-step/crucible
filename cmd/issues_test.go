package cmd

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
)

func testSynthesisResult() *domain.SynthesisResult {
	return &domain.SynthesisResult{
		Synthesizer: "ORACLE",
		Model:       "anthropic/claude-opus-4-6",
		Summary:     "Test synthesis",
		Items: []domain.SynthesisItem{
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
		},
	}
}

func writeSynthesisFixture(t *testing.T, dir string) string {
	t.Helper()
	data, err := json.MarshalIndent(testSynthesisResult(), "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "synthesis.json")
	if err := os.WriteFile(path, data, 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestCreateIssuesRequiresFlags(t *testing.T) {
	root := rootCmd
	root.SetOut(new(bytes.Buffer))
	root.SetErr(new(bytes.Buffer))
	root.SetArgs([]string{"create-issues"})

	err := root.Execute()
	if err == nil {
		t.Fatal("expected error when --repo and --input are missing")
	}
}

func TestCreateIssuesDryRun(t *testing.T) {
	dir := t.TempDir()
	inputPath := writeSynthesisFixture(t, dir)

	stdout := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(new(bytes.Buffer))
	root.SetArgs([]string{"create-issues", "--repo", "owner/repo", "--input", inputPath, "--dry-run"})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	output := stdout.String()
	if !strings.Contains(output, "Dry Run") {
		t.Fatalf("expected dry run output, got: %s", output)
	}
	if !strings.Contains(output, "Implement auth module") {
		t.Fatalf("expected item title in output, got: %s", output)
	}
}

func TestCreateIssuesInvalidJSON(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "bad.json")
	if err := os.WriteFile(path, []byte("not json"), 0o644); err != nil {
		t.Fatal(err)
	}

	root := rootCmd
	root.SetOut(new(bytes.Buffer))
	root.SetErr(new(bytes.Buffer))
	root.SetArgs([]string{"create-issues", "--repo", "owner/repo", "--input", path})

	err := root.Execute()
	if err == nil {
		t.Fatal("expected error for invalid JSON")
	}
	if !strings.Contains(err.Error(), "parse") {
		t.Fatalf("expected parse error, got: %v", err)
	}
}

func TestCreateIssuesMissingFile(t *testing.T) {
	root := rootCmd
	root.SetOut(new(bytes.Buffer))
	root.SetErr(new(bytes.Buffer))
	root.SetArgs([]string{"create-issues", "--repo", "owner/repo", "--input", "/nonexistent/file.json"})

	err := root.Execute()
	if err == nil {
		t.Fatal("expected error for missing file")
	}
	if !strings.Contains(err.Error(), "read input") {
		t.Fatalf("expected read error, got: %v", err)
	}
}

func TestCreateIssuesWithLabels(t *testing.T) {
	dir := t.TempDir()
	inputPath := writeSynthesisFixture(t, dir)

	stdout := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(new(bytes.Buffer))
	root.SetArgs([]string{"create-issues", "--repo", "owner/repo", "--input", inputPath, "--dry-run", "--labels", "extra-label,another"})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	output := stdout.String()
	if !strings.Contains(output, "extra-label") {
		t.Fatalf("expected extra label in output, got: %s", output)
	}
}
