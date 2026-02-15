package models

import (
	"testing"
	"time"

	"github.com/misty-step/crucible/internal/config"
)

func TestDefaultRegistryHasAllPerspectives(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	expected := []string{"product", "engineering", "design", "business", "synthesis"}

	for _, name := range expected {
		if _, ok := r.Get(name); !ok {
			t.Errorf("missing perspective %q", name)
		}
	}
}

func TestFromConfigCreatesRegistry(t *testing.T) {
	t.Parallel()

	cfgPerspectives := []config.PerspectiveConfig{
		{
			Name:    "test",
			Enabled: true,
			Timeout: 60 * time.Second,
			Model: config.ModelConfig{
				ID:       "provider/model",
				Provider: "provider",
				Name:     "model",
			},
			Fallbacks: []config.ModelConfig{
				{ID: "fallback/model", Provider: "fallback", Name: "fallback"},
			},
		},
	}

	r := FromConfig(cfgPerspectives)

	cfg, ok := r.Get("test")
	if !ok {
		t.Fatal("expected to find test perspective")
	}

	if cfg.Primary.ID != "provider/model" {
		t.Errorf("primary model = %q, want provider/model", cfg.Primary.ID)
	}

	if len(cfg.Fallbacks) != 1 {
		t.Errorf("fallbacks = %d, want 1", len(cfg.Fallbacks))
	}

	if cfg.Timeout != 60*time.Second {
		t.Errorf("timeout = %v, want 60s", cfg.Timeout)
	}
}

func TestFromConfigSkipsDisabledPerspectives(t *testing.T) {
	t.Parallel()

	cfgPerspectives := []config.PerspectiveConfig{
		{
			Name:    "enabled",
			Enabled: true,
			Timeout: 60 * time.Second,
			Model:   config.ModelConfig{ID: "provider/enabled", Provider: "p", Name: "n"},
		},
		{
			Name:    "disabled",
			Enabled: false,
			Timeout: 60 * time.Second,
			Model:   config.ModelConfig{ID: "provider/disabled", Provider: "p", Name: "n"},
		},
	}

	r := FromConfig(cfgPerspectives)

	if _, ok := r.Get("enabled"); !ok {
		t.Error("expected to find enabled perspective")
	}

	if _, ok := r.Get("disabled"); ok {
		t.Error("expected NOT to find disabled perspective")
	}
}

func TestFromConfigEmptyFallbacks(t *testing.T) {
	t.Parallel()

	cfgPerspectives := []config.PerspectiveConfig{
		{
			Name:      "nofallback",
			Enabled:   true,
			Timeout:   60 * time.Second,
			Model:     config.ModelConfig{ID: "provider/model", Provider: "p", Name: "n"},
			Fallbacks: nil,
		},
	}

	r := FromConfig(cfgPerspectives)

	cfg, ok := r.Get("nofallback")
	if !ok {
		t.Fatal("expected to find perspective")
	}

	if len(cfg.Fallbacks) != 0 {
		t.Errorf("expected 0 fallbacks, got %d", len(cfg.Fallbacks))
	}
}

func TestDefaultRegistryModelDiversity(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	providers := make(map[string]bool)

	for _, name := range []string{"product", "engineering", "design", "business"} {
		cfg, _ := r.Get(name)
		providers[cfg.Primary.Provider] = true
	}

	if len(providers) < 4 {
		t.Fatalf("got %d unique providers, want at least 4", len(providers))
	}
}

func TestSynthesisHasNoFallback(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	cfg, ok := r.Get("synthesis")
	if !ok {
		t.Fatal("missing synthesis perspective")
	}
	if len(cfg.Fallbacks) != 0 {
		t.Fatalf("synthesis should have no fallbacks, got %d", len(cfg.Fallbacks))
	}
}

func TestSynthesisTimeout(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	cfg, _ := r.Get("synthesis")
	if cfg.Timeout != 300*time.Second {
		t.Fatalf("synthesis timeout = %v, want 300s", cfg.Timeout)
	}
}

func TestNextModelPrimary(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	model, ok := r.NextModel("product", "")
	if !ok {
		t.Fatal("expected primary model")
	}
	if model.ID != "anthropic/claude-sonnet-4.5" {
		t.Fatalf("got %q, want anthropic/claude-sonnet-4.5", model.ID)
	}
}

func TestNextModelFallback(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	model, ok := r.NextModel("product", "anthropic/claude-sonnet-4.5")
	if !ok {
		t.Fatal("expected fallback model")
	}
	if model.ID != "google/gemini-3-flash-preview" {
		t.Fatalf("got %q, want google/gemini-3-flash-preview", model.ID)
	}
}

func TestNextModelExhausted(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	_, ok := r.NextModel("product", "google/gemini-3-flash-preview")
	if ok {
		t.Fatal("expected chain exhausted")
	}
}

func TestNextModelSynthesisExhausted(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	_, ok := r.NextModel("synthesis", "anthropic/claude-opus-4.6")
	if ok {
		t.Fatal("synthesis should have no fallback")
	}
}

func TestNextModelUnknownPerspective(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	_, ok := r.NextModel("unknown", "")
	if ok {
		t.Fatal("expected false for unknown perspective")
	}
}

func TestPerspectives(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	names := r.Perspectives()
	if len(names) != 5 {
		t.Fatalf("got %d perspectives, want 5", len(names))
	}
}
