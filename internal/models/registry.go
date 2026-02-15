// Package models provides model registry and configuration.
package models

import (
	"sort"
	"time"

	"github.com/misty-step/crucible/internal/config"
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
	return FromConfig(config.DefaultPerspectives())
}

// FromConfig creates a Registry from config perspectives.
func FromConfig(perspectives []config.PerspectiveConfig) *Registry {
	r := &Registry{
		perspectives: make(map[string]PerspectiveConfig),
	}

	for _, p := range perspectives {
		if !p.Enabled {
			continue
		}

		fallbacks := make([]Model, len(p.Fallbacks))
		for i, fb := range p.Fallbacks {
			fallbacks[i] = Model{
				ID:       fb.ID,
				Provider: fb.Provider,
				Name:     fb.Name,
			}
		}

		r.perspectives[p.Name] = PerspectiveConfig{
			Primary: Model{
				ID:       p.Model.ID,
				Provider: p.Model.Provider,
				Name:     p.Model.Name,
			},
			Fallbacks: fallbacks,
			Timeout:   p.Timeout,
		}
	}

	return r
}

// LoadRegistry loads a registry from default and local config files.
func LoadRegistry(defaultsPath, localPath string) (*Registry, error) {
	cfg, err := config.LoadMerged(defaultsPath, localPath)
	if err != nil {
		return nil, err
	}
	return FromConfig(cfg.Perspectives), nil
}

// Get returns the config for a perspective, or false if not found.
func (r *Registry) Get(perspective string) (PerspectiveConfig, bool) {
	cfg, ok := r.perspectives[perspective]
	return cfg, ok
}

// Perspectives returns all registered perspective names in sorted order.
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
