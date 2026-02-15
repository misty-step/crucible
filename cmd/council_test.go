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
