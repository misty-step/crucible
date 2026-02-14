package models

import (
	"sort"
	"time"
)

const SynthesisPerspective = "synthesis"

// Model identifies an LLM available via OpenRouter.
type Model struct {
	ID       string // e.g. "anthropic/claude-sonnet-4.5"
	Provider string // e.g. "anthropic"
	Name     string // e.g. "claude-sonnet-4.5"
}

// PerspectiveConfig maps a council perspective to its model chain.
type PerspectiveConfig struct {
	Primary   Model
	Fallbacks []Model
	Timeout   time.Duration
}

// Registry holds model configurations for all perspectives.
type Registry struct {
	perspectives map[string]PerspectiveConfig
}

// DefaultRegistry returns the standard model assignments for all 5 perspectives.
func DefaultRegistry() *Registry {
	return &Registry{
		perspectives: map[string]PerspectiveConfig{
			"product": {
				Primary:   Model{ID: "anthropic/claude-sonnet-4.5", Provider: "anthropic", Name: "claude-sonnet-4.5"},
				Fallbacks: []Model{{ID: "google/gemini-3-flash-preview", Provider: "google", Name: "gemini-3-flash-preview"}},
				Timeout:   120 * time.Second,
			},
			"engineering": {
				Primary:   Model{ID: "moonshotai/kimi-k2.5", Provider: "moonshotai", Name: "kimi-k2.5"},
				Fallbacks: []Model{{ID: "z-ai/glm-5", Provider: "z-ai", Name: "glm-5"}},
				Timeout:   120 * time.Second,
			},
			"design": {
				Primary:   Model{ID: "google/gemini-3-flash-preview", Provider: "google", Name: "gemini-3-flash-preview"},
				Fallbacks: []Model{{ID: "z-ai/glm-5", Provider: "z-ai", Name: "glm-5"}},
				Timeout:   120 * time.Second,
			},
			"business": {
				Primary:   Model{ID: "qwen/qwen3-max-thinking", Provider: "qwen", Name: "qwen3-max-thinking"},
				Fallbacks: []Model{{ID: "z-ai/glm-5", Provider: "z-ai", Name: "glm-5"}},
				Timeout:   120 * time.Second,
			},
			SynthesisPerspective: {
				Primary:   Model{ID: "anthropic/claude-opus-4.6", Provider: "anthropic", Name: "claude-opus-4.6"},
				Fallbacks: []Model{}, // No fallback — synthesis quality is non-negotiable
				Timeout:   300 * time.Second,
			},
		},
	}
}

// Get returns the config for a perspective, or false if not found.
func (r *Registry) Get(perspective string) (PerspectiveConfig, bool) {
	cfg, ok := r.perspectives[perspective]
	return cfg, ok
}

// Perspectives returns all registered perspective names.
func (r *Registry) Perspectives() []string {
	names := make([]string, 0, len(r.perspectives))
	for name := range r.perspectives {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}

// NextModel returns the next model to try after failedModelID, or false
// if the chain is exhausted. Pass "" as failedModelID to get the primary.
func (r *Registry) NextModel(perspective string, failedModelID string) (Model, bool) {
	cfg, ok := r.perspectives[perspective]
	if !ok {
		return Model{}, false
	}

	if failedModelID == "" {
		return cfg.Primary, true
	}

	if failedModelID == cfg.Primary.ID {
		if len(cfg.Fallbacks) > 0 {
			return cfg.Fallbacks[0], true
		}
		return Model{}, false
	}

	for i, fb := range cfg.Fallbacks {
		if fb.ID == failedModelID && i+1 < len(cfg.Fallbacks) {
			return cfg.Fallbacks[i+1], true
		}
	}

	return Model{}, false
}
