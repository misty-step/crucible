package config

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestDefaultPerspectives(t *testing.T) {
	t.Parallel()

	defaults := DefaultPerspectives()
	if len(defaults) != 5 {
		t.Fatalf("expected 5 default perspectives, got %d", len(defaults))
	}

	expected := map[string]bool{
		"product":     true,
		"engineering": true,
		"design":      true,
		"business":    true,
		"synthesis":   true,
	}

	for _, p := range defaults {
		if !expected[p.Name] {
			t.Errorf("unexpected perspective: %q", p.Name)
		}
		delete(expected, p.Name)
	}

	if len(expected) > 0 {
		for name := range expected {
			t.Errorf("missing perspective: %q", name)
		}
	}
}

func TestDefaultProductPerspective(t *testing.T) {
	t.Parallel()

	defaults := DefaultPerspectives()
	var product *PerspectiveConfig
	for i := range defaults {
		if defaults[i].Name == "product" {
			product = &defaults[i]
			break
		}
	}

	if product == nil {
		t.Fatal("product perspective not found")
	}

	if product.Model.ID != "anthropic/claude-sonnet-4.5" {
		t.Errorf("product model = %q, want anthropic/claude-sonnet-4.5", product.Model.ID)
	}

	if len(product.Fallbacks) != 1 {
		t.Errorf("product fallbacks = %d, want 1", len(product.Fallbacks))
	}

	if product.Fallbacks[0].ID != "google/gemini-3-flash-preview" {
		t.Errorf("product fallback = %q, want google/gemini-3-flash-preview", product.Fallbacks[0].ID)
	}

	if product.Timeout != 120*time.Second {
		t.Errorf("product timeout = %v, want 120s", product.Timeout)
	}

	if !product.Enabled {
		t.Error("product should be enabled")
	}
}

func TestDefaultSynthesisPerspective(t *testing.T) {
	t.Parallel()

	defaults := DefaultPerspectives()
	var synthesis *PerspectiveConfig
	for i := range defaults {
		if defaults[i].Name == "synthesis" {
			synthesis = &defaults[i]
			break
		}
	}

	if synthesis == nil {
		t.Fatal("synthesis perspective not found")
	}

	if synthesis.Model.ID != "anthropic/claude-opus-4.6" {
		t.Errorf("synthesis model = %q, want anthropic/claude-opus-4.6", synthesis.Model.ID)
	}

	if len(synthesis.Fallbacks) != 0 {
		t.Errorf("synthesis should have no fallbacks, got %d", len(synthesis.Fallbacks))
	}

	if synthesis.Timeout != 300*time.Second {
		t.Errorf("synthesis timeout = %v, want 300s", synthesis.Timeout)
	}
}

func TestLoadValidConfig(t *testing.T) {
	t.Parallel()

	content := `
perspectives:
  - name: custom
    agent: custom.md
    model:
      id: provider/model-name
      provider: provider
      name: model-name
    fallbacks:
      - id: fallback/model
        provider: fallback
        name: fallback-name
    timeout: 60s
    enabled: true
`
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yml")
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}

	cfg, err := Load(path)
	if err != nil {
		t.Fatalf("load config: %v", err)
	}

	if len(cfg.Perspectives) != 1 {
		t.Fatalf("expected 1 perspective, got %d", len(cfg.Perspectives))
	}

	p := cfg.Perspectives[0]
	if p.Name != "custom" {
		t.Errorf("name = %q, want custom", p.Name)
	}
	if p.Model.ID != "provider/model-name" {
		t.Errorf("model ID = %q, want provider/model-name", p.Model.ID)
	}
	if p.Timeout != 60*time.Second {
		t.Errorf("timeout = %v, want 60s", p.Timeout)
	}
}

func TestLoadNonExistentFile(t *testing.T) {
	t.Parallel()

	_, err := Load("/nonexistent/path.yml")
	if err == nil {
		t.Fatal("expected error for non-existent file")
	}

	if !os.IsNotExist(err) {
		t.Errorf("expected IsNotExist, got: %v", err)
	}
}

func TestLoadOrDefaultReturnsDefaults(t *testing.T) {
	t.Parallel()

	cfg, err := LoadOrDefault("/nonexistent/path.yml")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(cfg.Perspectives) != 5 {
		t.Errorf("expected 5 perspectives, got %d", len(cfg.Perspectives))
	}
}

func TestLoadOrDefaultLoadsExistingFile(t *testing.T) {
	t.Parallel()

	content := `
perspectives:
  - name: single
    agent: single.md
    model:
      id: provider/single
      provider: provider
      name: single
    enabled: true
`
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yml")
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}

	cfg, err := LoadOrDefault(path)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(cfg.Perspectives) != 1 {
		t.Errorf("expected 1 perspective, got %d", len(cfg.Perspectives))
	}
}

func TestMergePreservesDefaults(t *testing.T) {
	t.Parallel()

	defaults := DefaultPerspectives()
	local := []PerspectiveConfig{}

	merged := Merge(defaults, local)

	if len(merged) != 5 {
		t.Errorf("expected 5 perspectives, got %d", len(merged))
	}
}

func TestMergeDisablesPerspective(t *testing.T) {
	t.Parallel()

	defaults := []PerspectiveConfig{
		{Name: "product", Enabled: true, Timeout: 120 * time.Second},
		{Name: "design", Enabled: true, Timeout: 120 * time.Second},
	}

	local := []PerspectiveConfig{
		{Name: "product", Enabled: false, Timeout: 120 * time.Second},
	}

	merged := Merge(defaults, local)

	if len(merged) != 1 {
		t.Fatalf("expected 1 perspective, got %d", len(merged))
	}

	if merged[0].Name != "design" {
		t.Errorf("expected design, got %q", merged[0].Name)
	}
}

func TestMergeAddsNewPerspective(t *testing.T) {
	t.Parallel()

	defaults := []PerspectiveConfig{
		{Name: "product", Enabled: true, Timeout: 120 * time.Second},
	}

	local := []PerspectiveConfig{
		{Name: "security", Enabled: true, Timeout: 60 * time.Second, Agent: "security.md"},
	}

	merged := Merge(defaults, local)

	if len(merged) != 2 {
		t.Fatalf("expected 2 perspectives, got %d", len(merged))
	}

	names := make(map[string]bool)
	for _, p := range merged {
		names[p.Name] = true
	}

	if !names["product"] {
		t.Error("product perspective missing")
	}
	if !names["security"] {
		t.Error("security perspective missing")
	}
}

func TestMergeOverridesValues(t *testing.T) {
	t.Parallel()

	defaults := []PerspectiveConfig{
		{
			Name:    "product",
			Enabled: true,
			Agent:   "default.md",
			Timeout: 120 * time.Second,
			Model: ModelConfig{
				ID:       "default/model",
				Provider: "default",
				Name:     "model",
			},
		},
	}

	local := []PerspectiveConfig{
		{
			Name:    "product",
			Enabled: true,
			Agent:   "custom.md",
			Timeout: 60 * time.Second,
			Model: ModelConfig{
				ID:       "custom/model",
				Provider: "custom",
				Name:     "custom",
			},
		},
	}

	merged := Merge(defaults, local)

	if len(merged) != 1 {
		t.Fatalf("expected 1 perspective, got %d", len(merged))
	}

	p := merged[0]
	if p.Agent != "custom.md" {
		t.Errorf("agent = %q, want custom.md", p.Agent)
	}
	if p.Timeout != 60*time.Second {
		t.Errorf("timeout = %v, want 60s", p.Timeout)
	}
	if p.Model.ID != "custom/model" {
		t.Errorf("model ID = %q, want custom/model", p.Model.ID)
	}
}

func TestFindLocalConfigInCurrentDir(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	configPath := filepath.Join(dir, ".crucible.yml")
	if err := os.WriteFile(configPath, []byte("test"), 0o644); err != nil {
		t.Fatal(err)
	}

	found, ok := FindLocalConfig(dir)
	if !ok {
		t.Fatal("expected to find config")
	}
	if found != configPath {
		t.Errorf("found = %q, want %q", found, configPath)
	}
}

func TestFindLocalConfigInParentDir(t *testing.T) {
	t.Parallel()

	parentDir := t.TempDir()
	subDir := filepath.Join(parentDir, "subdir")
	if err := os.MkdirAll(subDir, 0o755); err != nil {
		t.Fatal(err)
	}

	configPath := filepath.Join(parentDir, ".crucible.yml")
	if err := os.WriteFile(configPath, []byte("test"), 0o644); err != nil {
		t.Fatal(err)
	}

	found, ok := FindLocalConfig(subDir)
	if !ok {
		t.Fatal("expected to find config in parent")
	}
	if found != configPath {
		t.Errorf("found = %q, want %q", found, configPath)
	}
}

func TestFindLocalConfigNotFound(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	_, ok := FindLocalConfig(dir)
	if ok {
		t.Error("expected not to find config")
	}
}

func TestLoadMergedWithDefaultsOnly(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	defaultsPath := filepath.Join(dir, "defaults.yml")

	// Create defaults file
	defaults := `
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
	if err := os.WriteFile(defaultsPath, []byte(defaults), 0o644); err != nil {
		t.Fatal(err)
	}

	localPath := filepath.Join(dir, ".crucible.yml")

	cfg, err := LoadMerged(defaultsPath, localPath)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(cfg.Perspectives) != 1 {
		t.Errorf("expected 1 perspective, got %d", len(cfg.Perspectives))
	}
}

func TestLoadMergedWithLocalOverride(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	defaultsPath := filepath.Join(dir, "defaults.yml")
	localPath := filepath.Join(dir, ".crucible.yml")

	defaults := `
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
	if err := os.WriteFile(defaultsPath, []byte(defaults), 0o644); err != nil {
		t.Fatal(err)
	}

	local := `
perspectives:
  - name: product
    agent: custom-product.md
    timeout: 180s
    enabled: true
`
	if err := os.WriteFile(localPath, []byte(local), 0o644); err != nil {
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
	if p.Agent != "custom-product.md" {
		t.Errorf("agent = %q, want custom-product.md", p.Agent)
	}
	if p.Timeout != 180*time.Second {
		t.Errorf("timeout = %v, want 180s", p.Timeout)
	}
}

func TestLoadMergedWithoutDefaultsFile(t *testing.T) {
	t.Parallel()

	dir := t.TempDir()
	defaultsPath := filepath.Join(dir, "nonexistent.yml")
	localPath := filepath.Join(dir, ".crucible.yml")

	cfg, err := LoadMerged(defaultsPath, localPath)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Should use Go defaults (5 perspectives)
	if len(cfg.Perspectives) != 5 {
		t.Errorf("expected 5 perspectives, got %d", len(cfg.Perspectives))
	}
}