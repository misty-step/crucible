package cmd

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCouncilRequiresRepo(t *testing.T) {
	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetArgs([]string{"council"})

	err := root.Execute()
	if err == nil {
		t.Fatal("expected error when --repo is missing")
	}
}

func TestCouncilDryRun(t *testing.T) {
	repoDir := t.TempDir()

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetArgs([]string{"council", "--repo", repoDir, "--dry-run"})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stdout.String(), "dry-run") {
		t.Fatalf("expected dry-run output, got: %s", stdout.String())
	}
}

func TestCouncilDryRunVerbose(t *testing.T) {
	repoDir := t.TempDir()

	if err := os.WriteFile(filepath.Join(repoDir, "VISION.md"), []byte("Test vision"), 0o644); err != nil {
		t.Fatal(err)
	}

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetArgs([]string{"council", "--repo", repoDir, "--dry-run", "--verbose", "--vision", filepath.Join(repoDir, "VISION.md")})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "Gathering repository context") {
		t.Errorf("expected verbose output on stderr, got: %s", stderr.String())
	}
}

func TestCouncilInteractive(t *testing.T) {
	repoDir := t.TempDir()
	visionPath := filepath.Join(repoDir, "VISION.md")

	if err := os.WriteFile(visionPath, []byte("Test vision content"), 0o644); err != nil {
		t.Fatal(err)
	}

	// Simulate interactive input from user
	input := "Prioritize reliability\nFocus on testing\n\n"
	stdin := strings.NewReader(input)

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetIn(stdin)
	root.SetArgs([]string{"council", "--repo", repoDir, "--dry-run", "--interactive", "--vision", visionPath})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	output := stdout.String()

	// Verify context summary is printed
	if !strings.Contains(output, "Repository Context Summary") {
		t.Errorf("expected context summary, got: %s", output)
	}

	// Verify vision is shown
	if !strings.Contains(output, "Vision:") {
		t.Errorf("expected vision header in output, got: %s", output)
	}

	// Verify human input prompt appears
	if !strings.Contains(output, "What priorities, concerns, or ideas") {
		t.Errorf("expected human input prompt, got: %s", output)
	}

	// Verify dry-run completes
	if !strings.Contains(output, "dry-run") {
		t.Errorf("expected dry-run output, got: %s", output)
	}

	t.Logf("stdout: %s", output)
	t.Logf("stderr: %s", stderr.String())
}

func TestCouncilInteractiveMultiLine(t *testing.T) {
	repoDir := t.TempDir()

	// Simulate multi-line input ending with empty line
	input := "Line one\nLine two\nLine three\n\n"
	stdin := strings.NewReader(input)

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetIn(stdin)
	root.SetArgs([]string{"council", "--repo", repoDir, "--dry-run", "--interactive"})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Verify council completes
	if !strings.Contains(stdout.String(), "dry-run") {
		t.Errorf("expected dry-run output, got: %s", stdout.String())
	}
}
