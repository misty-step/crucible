package cmd

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCouncilWithCustomConfig(t *testing.T) {
	t.Parallel()

	repoDir := t.TempDir()
	configDir := t.TempDir()

	// Create a custom config with only 2 perspectives
	configContent := `
perspectives:
  - name: product
    agent: product.md
    model:
      id: anthropic/claude-sonnet-4.5
      provider: anthropic
      name: claude-sonnet-4.5
    timeout: 120s
    enabled: true
  - name: engineering
    agent: engineering.md
    model:
      id: moonshotai/kimi-k2.5
      provider: moonshotai
      name: kimi-k2.5
    timeout: 120s
    enabled: true
`
	configPath := filepath.Join(configDir, "custom-config.yml")
	if err := os.WriteFile(configPath, []byte(configContent), 0o644); err != nil {
		t.Fatal(err)
	}

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetArgs([]string{"council", "--repo", repoDir, "--dry-run", "--config", configPath})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stdout.String(), "dry-run") {
		t.Errorf("expected dry-run output, got: %s", stdout.String())
	}
}

func TestCouncilWithNonexistentConfig(t *testing.T) {
	t.Parallel()

	repoDir := t.TempDir()

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetArgs([]string{"council", "--repo", repoDir, "--config", "/nonexistent/config.yml"})

	err := root.Execute()
	// When explicit config path is provided and file doesn't exist,
	// LoadOrDefault returns the error (not falling back to defaults)
	if err == nil {
		t.Fatal("expected error for nonexistent config")
	}
}
