package config

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestIntegrationWithDefaultsFile(t *testing.T) {
	t.Parallel()

	// Create a temp dir structure like the real project
	dir := t.TempDir()
	defaultsDir := filepath.Join(dir, "defaults")
	if err := os.MkdirAll(defaultsDir, 0o755); err != nil {
		t.Fatal(err)
	}

	defaultsContent := `
perspectives:
  - name: product
    agent: product.md
    model:
      id: anthropic/claude-sonnet-4.5
      provider: anthropic
      name: claude-sonnet-4.5
    fallbacks:
      - id: google/gemini-3-flash-preview
        provider: google
        name: gemini-3-flash-preview
    timeout: 120s
    enabled: true
  - name: engineering
    agent: engineering.md
    model:
      id: moonshotai/kimi-k2.5
      provider: moonshotai
      name: kimi-k2.5
    fallbacks: []
    timeout: 120s
    enabled: true
`
	defaultsPath := filepath.Join(defaultsDir, "config.yml")
	if err := os.WriteFile(defaultsPath, []byte(defaultsContent), 0o644); err != nil {
		t.Fatal(err)
	}

	// Test with local override that disables engineering
	repoDir := filepath.Join(dir, "repo")
	if err := os.MkdirAll(repoDir, 0o755); err != nil {
		t.Fatal(err)
	}

	localContent := `
perspectives:
  - name: engineering
    enabled: false
`
	localPath := filepath.Join(repoDir, ".crucible.yml")
	if err := os.WriteFile(localPath, []byte(localContent), 0o644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadMerged(defaultsPath, localPath)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should only have product (engineering disabled)
	if len(cfg.Perspectives) != 1 {
		t.Errorf("expected 1 perspective, got %d", len(cfg.Perspectives))
	}

	if cfg.Perspectives[0].Name != "product" {
		t.Errorf("expected product, got %q", cfg.Perspectives[0].Name)
	}
}

func TestIntegrationWithNewPerspective(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	defaultsDir := filepath.Join(dir, "defaults")
	if err := os.MkdirAll(defaultsDir, 0o755); err != nil {
		t.Fatal(err)
	}

	defaultsContent := `
perspectives:
  - name: product
    agent: product.md
    model:
      id: anthropic/claude-sonnet-4.5
      provider: anthropic
      name: claude-sonnet-4.5
    timeout: 120s
    enabled: true
`
	defaultsPath := filepath.Join(defaultsDir, "config.yml")
	if err := os.WriteFile(defaultsPath, []byte(defaultsContent), 0o644); err != nil {
		t.Fatal(err)
	}

	// Add security perspective locally
	localContent := `
perspectives:
  - name: security
    agent: security.md
    model:
      id: anthropic/claude-opus-4.6
      provider: anthropic
      name: claude-opus-4.6
    timeout: 180s
    enabled: true
`
	localPath := filepath.Join(dir, ".crucible.yml")
	if err := os.WriteFile(localPath, []byte(localContent), 0o644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadMerged(defaultsPath, localPath)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should have both product and security
	if len(cfg.Perspectives) != 2 {
		t.Errorf("expected 2 perspectives, got %d", len(cfg.Perspectives))
	}

	names := make(map[string]bool)
	for _, p := range cfg.Perspectives {
		names[p.Name] = true
	}

	if !names["product"] {
		t.Error("product perspective missing")
	}
	if !names["security"] {
		t.Error("security perspective missing")
	}

	// Verify security has custom timeout
	for _, p := range cfg.Perspectives {
		if p.Name == "security" {
			if p.Timeout != 180*time.Second {
				t.Errorf("security timeout = %v, want 180s", p.Timeout)
			}
		}
	}
}

func TestIntegrationModelOverride(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	defaultsPath := filepath.Join(dir, "defaults.yml")

	defaultsContent := `
perspectives:
  - name: product
    agent: product.md
    model:
      id: original/model
      provider: original
      name: original-model
    timeout: 120s
    enabled: true
`
	if err := os.WriteFile(defaultsPath, []byte(defaultsContent), 0o644); err != nil {
		t.Fatal(err)
	}

	// Override just the model
	localContent := `
perspectives:
  - name: product
    model:
      id: overridden/model
      provider: overridden
      name: overridden-model
    enabled: true
`
	localPath := filepath.Join(dir, ".crucible.yml")
	if err := os.WriteFile(localPath, []byte(localContent), 0o644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadMerged(defaultsPath, localPath)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(cfg.Perspectives) != 1 {
		t.Fatalf("expected 1 perspective, got %d", len(cfg.Perspectives))
	}

	p := cfg.Perspectives[0]
	if p.Model.ID != "overridden/model" {
		t.Errorf("model ID = %q, want overridden/model", p.Model.ID)
	}

	// Agent should still be from defaults since not overridden
	if p.Agent != "product.md" {
		t.Errorf("agent = %q, want product.md", p.Agent)
	}
}
