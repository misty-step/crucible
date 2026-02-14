package council

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
	"github.com/misty-step/crucible/internal/models"
)

const (
	maxRetries      = 3
	initialBackoff  = 2 * time.Second
	minPerspectives = 2
)

// Spawner runs council agents in parallel with retry and fallback.
type Spawner struct {
	Runner   cruxexec.CommandRunner
	Registry *models.Registry
	Env      []string // pre-filtered env vars for child processes
}

// NewSpawner creates a Spawner with filtered environment variables.
func NewSpawner(runner cruxexec.CommandRunner, registry *models.Registry) *Spawner {
	return &Spawner{
		Runner:   runner,
		Registry: registry,
		Env:      cruxexec.FilterEnv(os.Environ(), cruxexec.AllowedEnvKeys),
	}
}

// RunCouncil spawns all council perspectives in parallel and returns results.
// Requires at least minPerspectives to succeed; returns error otherwise.
func (s *Spawner) RunCouncil(ctx context.Context, input domain.CouncilInput) ([]domain.SpawnResult, error) {
	perspectives := s.councilPerspectives()
	prompt := RenderPrompt(input)

	var (
		mu      sync.Mutex
		wg      sync.WaitGroup
		results = make([]domain.SpawnResult, len(perspectives))
	)

	for i, perspective := range perspectives {
		wg.Add(1)
		go func(idx int, persp string) {
			defer wg.Done()
			result := s.runPerspective(ctx, persp, prompt)
			mu.Lock()
			results[idx] = result
			mu.Unlock()
		}(i, perspective)
	}

	wg.Wait()

	succeeded := 0
	for _, r := range results {
		if r.Output != nil {
			succeeded++
		}
	}

	if succeeded < minPerspectives {
		return results, fmt.Errorf("council failed: only %d/%d perspectives succeeded (minimum %d)",
			succeeded, len(perspectives), minPerspectives)
	}

	return results, nil
}

// councilPerspectives returns non-synthesis perspective names.
func (s *Spawner) councilPerspectives() []string {
	var council []string
	for _, name := range s.Registry.Perspectives() {
		if name != models.SynthesisPerspective {
			council = append(council, name)
		}
	}
	return council
}

// runPerspective tries the primary model then fallbacks for a single perspective.
func (s *Spawner) runPerspective(ctx context.Context, perspective string, prompt string) domain.SpawnResult {
	start := time.Now()

	cfg, ok := s.Registry.Get(perspective)
	if !ok {
		return domain.SpawnResult{
			Skipped: true,
			Error:   fmt.Errorf("unknown perspective %q", perspective),
		}
	}

	// Try primary model with retries
	model := cfg.Primary
	totalRetries := 0

	for {
		result := s.tryModelWithRetries(ctx, perspective, model, prompt, cfg.Timeout)
		totalRetries += result.Retries

		if result.Output != nil {
			result.Duration = time.Since(start)
			result.Retries = totalRetries
			return result
		}

		if errors.Is(result.Error, ctx.Err()) {
			return domain.SpawnResult{
				Error:    result.Error,
				Model:    model.ID,
				Retries:  totalRetries,
				Duration: time.Since(start),
			}
		}

		// Try next fallback
		next, hasNext := s.Registry.NextModel(perspective, model.ID)
		if !hasNext {
			return domain.SpawnResult{
				Skipped:  true,
				Error:    fmt.Errorf("all models exhausted for %s: %v", perspective, result.Error),
				Model:    model.ID,
				Retries:  totalRetries,
				Duration: time.Since(start),
			}
		}
		model = next
	}
}

// tryModelWithRetries attempts a model up to maxRetries times with exponential backoff.
func (s *Spawner) tryModelWithRetries(ctx context.Context, perspective string, model models.Model, prompt string, timeout time.Duration) domain.SpawnResult {
	backoff := initialBackoff
	var lastErr error

	for attempt := 0; attempt <= maxRetries; attempt++ {
		if attempt > 0 {
			select {
			case <-ctx.Done():
				return domain.SpawnResult{Error: ctx.Err(), Model: model.ID, Retries: attempt}
			case <-time.After(backoff):
				backoff *= 2
			}
		}

		output, err := s.invokeAgent(ctx, perspective, model, prompt, timeout)
		if err == nil {
			return domain.SpawnResult{Output: output, Model: model.ID, Retries: attempt}
		}

		lastErr = err

		// Permanent errors don't retry
		if isPermanentError(err) {
			return domain.SpawnResult{Error: err, Model: model.ID, Retries: attempt}
		}
	}

	return domain.SpawnResult{Error: lastErr, Model: model.ID, Retries: maxRetries}
}

// invokeAgent runs opencode with the given agent and model, parses JSON output.
func (s *Spawner) invokeAgent(ctx context.Context, perspective string, model models.Model, prompt string, timeout time.Duration) (*domain.CouncilOutput, error) {
	sanitizedPrompt := cruxexec.SanitizeArg(prompt)
	args := []string{
		"run",
		"--agent", perspective,
		"-m", "openrouter/" + model.ID,
		sanitizedPrompt,
	}

	result, err := s.Runner.Run(ctx, "opencode", args, cruxexec.RunOpts{
		Env:     s.Env,
		Timeout: timeout,
	})
	if err != nil {
		return nil, fmt.Errorf("opencode %s: %w", perspective, err)
	}

	if result.ExitCode != 0 {
		stderr := string(result.Stderr)
		return nil, categorizeError(perspective, result.ExitCode, stderr)
	}

	// Extract JSON from output (may be wrapped in markdown code fences)
	jsonBytes := ExtractJSON(result.Stdout)
	if jsonBytes == nil {
		return nil, fmt.Errorf("opencode %s: no JSON found in output", perspective)
	}

	var output domain.CouncilOutput
	if err := json.Unmarshal(jsonBytes, &output); err != nil {
		return nil, fmt.Errorf("opencode %s: invalid JSON: %w", perspective, err)
	}

	if err := output.Validate(); err != nil {
		return nil, fmt.Errorf("opencode %s: %w", perspective, err)
	}

	return &output, nil
}

// extractJSON finds the first JSON object in output, handling markdown code fences.
func ExtractJSON(data []byte) []byte {
	tryDecode := func(raw []byte) []byte {
		var value json.RawMessage
		dec := json.NewDecoder(bytes.NewReader(bytes.TrimSpace(raw)))
		if err := dec.Decode(&value); err != nil {
			return nil
		}
		return []byte(value)
	}

	decodeFrom := func(start int) []byte {
		for start < len(data) {
			i := bytes.IndexByte(data[start:], '{')
			if i == -1 {
				return nil
			}
			i += start
			if decoded := tryDecode(data[i:]); decoded != nil {
				return decoded
			}
			start = i + 1
		}
		return nil
	}

	// Try to find JSON in fenced output
	if idx := bytes.Index(data, []byte("```json")); idx != -1 {
		if decoded := decodeFrom(idx + len("```json")); decoded != nil {
			return decoded
		}
	}

	// Try bare JSON object
	return decodeFrom(0)

}

// Error classification

type permanentError struct{ error }

func isPermanentError(err error) bool {
	var pe *permanentError
	return errors.As(err, &pe)
}

func categorizeError(perspective string, exitCode int, stderr string) error {
	msg := fmt.Sprintf("opencode %s exited %d: %s", perspective, exitCode, truncate(stderr, 200))

	// 401/403 patterns in stderr indicate auth issues (permanent)
	lower := strings.ToLower(stderr)
	if strings.Contains(lower, "401") || strings.Contains(lower, "403") ||
		strings.Contains(lower, "unauthorized") || strings.Contains(lower, "forbidden") {
		return &permanentError{fmt.Errorf("%s (permanent: auth failure)", msg)}
	}

	// Everything else is transient (retryable)
	return fmt.Errorf("%s", msg)
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
