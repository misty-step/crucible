package models

import (
	"testing"
	"time"
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
	if model.ID != "anthropic/claude-sonnet-4-5" {
		t.Fatalf("got %q, want anthropic/claude-sonnet-4-5", model.ID)
	}
}

func TestNextModelFallback(t *testing.T) {
	t.Parallel()

	r := DefaultRegistry()
	model, ok := r.NextModel("product", "anthropic/claude-sonnet-4-5")
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
	_, ok := r.NextModel("synthesis", "anthropic/claude-opus-4-6")
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
