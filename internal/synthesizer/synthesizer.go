package synthesizer

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/misty-step/crucible/internal/council"
	"github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
	"github.com/misty-step/crucible/internal/models"
)

// Service reconciles council outputs into a unified backlog via ORACLE.
type Service struct {
	Runner   cruxexec.CommandRunner
	Registry *models.Registry
	Env      []string
}

// NewService creates a synthesis service with filtered environment.
func NewService(runner cruxexec.CommandRunner, registry *models.Registry) *Service {
	return &Service{
		Runner:   runner,
		Registry: registry,
		Env:      cruxexec.FilterEnv(os.Environ(), cruxexec.AllowedEnvKeys),
	}
}

// Synthesize runs ORACLE to merge council outputs against the vision.
func (s *Service) Synthesize(ctx context.Context, input domain.SynthesisInput) (*domain.SynthesisResult, error) {
	cfg, ok := s.Registry.Get(models.SynthesisPerspective)
	if !ok {
		return nil, fmt.Errorf("synthesis perspective not found in registry")
	}

	prompt := RenderSynthesisPrompt(input)

	sanitizedPrompt := cruxexec.SanitizeArg(prompt)

	args := []string{
		"run",
		"--agent", "synthesis",
		"-m", "openrouter/" + cfg.Primary.ID,
		sanitizedPrompt,
	}

	start := time.Now()
	result, err := s.Runner.Run(ctx, "opencode", args, cruxexec.RunOpts{
		Env:     s.Env,
		Timeout: cfg.Timeout,
	})
	if err != nil {
		return nil, fmt.Errorf("synthesis failed: %w (took %v)", err, time.Since(start))
	}

	if result.ExitCode != 0 {
		return nil, fmt.Errorf("synthesis exited %d: %s", result.ExitCode, truncate(string(result.Stderr), 200))
	}

	jsonBytes := council.ExtractJSON(result.Stdout)
	if jsonBytes == nil {
		return nil, fmt.Errorf("synthesis: no JSON found in output")
	}

	var synth domain.SynthesisResult
	if err := json.Unmarshal(jsonBytes, &synth); err != nil {
		return nil, fmt.Errorf("synthesis: invalid JSON: %w", err)
	}

	if err := synth.Validate(); err != nil {
		return nil, fmt.Errorf("synthesis: %w", err)
	}

	return &synth, nil
}

func truncate(s string, maxLen int) string {
	s = strings.TrimSpace(s)
	if maxLen <= 0 {
		return ""
	}
	runes := []rune(s)
	if len(runes) > maxLen {
		return string(runes[:maxLen]) + "..."
	}
	return s
}
