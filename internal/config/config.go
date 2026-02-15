// Package config provides YAML-based configuration loading for perspectives.
package config

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"gopkg.in/yaml.v3"
)

// ModelConfig represents a single model configuration.
type ModelConfig struct {
	ID       string `yaml:"id"`
	Provider string `yaml:"provider"`
	Name     string `yaml:"name"`
}

// PerspectiveConfig represents a single perspective configuration.
type PerspectiveConfig struct {
	Name      string        `yaml:"name"`
	Agent     string        `yaml:"agent"`
	Model     ModelConfig   `yaml:"model"`
	Fallbacks []ModelConfig `yaml:"fallbacks,omitempty"`
	Timeout   time.Duration `yaml:"timeout"`
	Enabled   bool          `yaml:"enabled"`
}

// Config is the root configuration structure.
type Config struct {
	Perspectives []PerspectiveConfig `yaml:"perspectives"`
}

// DefaultPerspectives returns the default 4 perspectives matching current hardcoded values.
func DefaultPerspectives() []PerspectiveConfig {
	return []PerspectiveConfig{
		{
			Name:  "product",
			Agent: "product.md",
			Model: ModelConfig{
				ID:       "anthropic/claude-sonnet-4.5",
				Provider: "anthropic",
				Name:     "claude-sonnet-4.5",
			},
			Fallbacks: []ModelConfig{
				{ID: "google/gemini-3-flash-preview", Provider: "google", Name: "gemini-3-flash-preview"},
			},
			Timeout: 120 * time.Second,
			Enabled: true,
		},
		{
			Name:  "engineering",
			Agent: "engineering.md",
			Model: ModelConfig{
				ID:       "moonshotai/kimi-k2.5",
				Provider: "moonshotai",
				Name:     "kimi-k2.5",
			},
			Fallbacks: []ModelConfig{
				{ID: "z-ai/glm-5", Provider: "z-ai", Name: "glm-5"},
			},
			Timeout: 120 * time.Second,
			Enabled: true,
		},
		{
			Name:  "design",
			Agent: "design.md",
			Model: ModelConfig{
				ID:       "google/gemini-3-flash-preview",
				Provider: "google",
				Name:     "gemini-3-flash-preview",
			},
			Fallbacks: []ModelConfig{
				{ID: "z-ai/glm-5", Provider: "z-ai", Name: "glm-5"},
			},
			Timeout: 120 * time.Second,
			Enabled: true,
		},
		{
			Name:  "business",
			Agent: "business.md",
			Model: ModelConfig{
				ID:       "qwen/qwen3-max-thinking",
				Provider: "qwen",
				Name:     "qwen3-max-thinking",
			},
			Fallbacks: []ModelConfig{
				{ID: "z-ai/glm-5", Provider: "z-ai", Name: "glm-5"},
			},
			Timeout: 120 * time.Second,
			Enabled: true,
		},
		{
			Name:  "synthesis",
			Agent: "synthesis.md",
			Model: ModelConfig{
				ID:       "anthropic/claude-opus-4.6",
				Provider: "anthropic",
				Name:     "claude-opus-4.6",
			},
			Fallbacks: nil,
			Timeout:   300 * time.Second,
			Enabled:   true,
		},
	}
}

// Load reads a config file from the given path.
func Load(path string) (*Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}

	var cfg Config
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parse config: %w", err)
	}

	// Set defaults for perspectives
	for i := range cfg.Perspectives {
		if cfg.Perspectives[i].Timeout == 0 {
			cfg.Perspectives[i].Timeout = 120 * time.Second
		}
		// Default to enabled if not specified
		// Note: zero value is false, so we need a way to detect unset
		// For now, we require explicit enabled field
	}

	return &cfg, nil
}

// LoadOrDefault tries to load from path, returns defaults if file doesn't exist.
func LoadOrDefault(path string) (*Config, error) {
	cfg, err := Load(path)
	if err != nil {
		if os.IsNotExist(err) {
			return &Config{Perspectives: DefaultPerspectives()}, nil
		}
		return nil, fmt.Errorf("load config: %w", err)
	}
	return cfg, nil
}

// Merge combines local config overrides with defaults.
// Local perspectives override defaults by name.
// Disabled perspectives in local are excluded from result.
func Merge(defaults, local []PerspectiveConfig) []PerspectiveConfig {
	// Build map of local perspectives by name
	localMap := make(map[string]PerspectiveConfig)
	for _, p := range local {
		localMap[p.Name] = p
	}

	var result []PerspectiveConfig
	seen := make(map[string]bool)

	// Apply defaults with local overrides
	for _, d := range defaults {
		if local, ok := localMap[d.Name]; ok {
			// Skip if explicitly disabled
			if !local.Enabled {
				continue
			}
			// Merge: use local values where set, default where not
			merged := mergePerspective(d, local)
			result = append(result, merged)
			seen[d.Name] = true
		} else {
			// Use default as-is
			if d.Enabled {
				result = append(result, d)
			}
			seen[d.Name] = true
		}
	}

	// Add new perspectives from local that weren't in defaults
	for name, local := range localMap {
		if !seen[name] && local.Enabled {
			result = append(result, local)
		}
	}

	return result
}

func mergePerspective(def, local PerspectiveConfig) PerspectiveConfig {
	result := def

	if local.Agent != "" {
		result.Agent = local.Agent
	}
	if local.Model.ID != "" {
		result.Model = local.Model
	}
	if len(local.Fallbacks) > 0 {
		result.Fallbacks = local.Fallbacks
	}
	if local.Timeout != 0 {
		result.Timeout = local.Timeout
	}
	result.Enabled = local.Enabled

	return result
}

// LoadMerged loads defaults, then applies local overrides from .crucible.yml.
func LoadMerged(defaultsPath, localPath string) (*Config, error) {
	var defaults []PerspectiveConfig

	// Try to load defaults if file exists
	cfg, err := Load(defaultsPath)
	if err != nil {
		if os.IsNotExist(err) {
			defaults = DefaultPerspectives()
		} else {
			return nil, fmt.Errorf("load defaults: %w", err)
		}
	} else {
		defaults = cfg.Perspectives
	}

	var local []PerspectiveConfig
	localCfg, err := Load(localPath)
	if err != nil {
		if !os.IsNotExist(err) {
			return nil, fmt.Errorf("load local config: %w", err)
		}
		// If local doesn't exist, just use defaults
	} else {
		local = localCfg.Perspectives
	}

	return &Config{Perspectives: Merge(defaults, local)}, nil
}

// FindLocalConfig searches for .crucible.yml in current dir and parent dirs.
func FindLocalConfig(startDir string) (string, bool) {
	dir := startDir
	for {
		path := filepath.Join(dir, ".crucible.yml")
		if _, err := os.Stat(path); err == nil {
			return path, true
		}

		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	return "", false
}
