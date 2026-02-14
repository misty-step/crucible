package synthesizer

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
	"github.com/misty-step/crucible/internal/models"
)

func validSynthesisJSON() []byte {
	result := domain.SynthesisResult{
		Synthesizer: "ORACLE",
		Model:       "anthropic/claude-opus-4-6",
		Summary:     "Focused backlog for MVP",
		Items: []domain.SynthesisItem{
			{
				Title:    "Implement council spawner",
				Priority: domain.P0,
				Type:     domain.Feature,
				Horizon:  domain.Now,
				Effort:   domain.Large,
				Body:     "## Problem\n\nCouncil is a stub.\n\n## Impact\n\nBlocks everything.",
				Labels:   []string{"domain/council", "source/groom"},
				CouncilSupport: domain.CouncilSupport{
					ProposedBy: []string{"STRATEGIST", "ARCHITECT"},
					Consensus:  domain.Strong,
				},
			},
		},
		Dropped: []domain.DroppedItem{
			{Title: "Add dark mode", Reason: "Not aligned with vision"},
		},
	}
	data, _ := json.Marshal(result)
	return data
}

func TestSynthesizeSuccess(t *testing.T) {
	t.Parallel()

	mock := cruxexec.NewMockRunner()
	mock.Results["opencode"] = &cruxexec.RunResult{
		Stdout:   validSynthesisJSON(),
		ExitCode: 0,
	}

	svc := &Service{
		Runner:   mock,
		Registry: models.DefaultRegistry(),
		Env:      []string{"HOME=/tmp"},
	}

	input := domain.SynthesisInput{
		CouncilOutputs: []domain.CouncilOutput{
			{Councilor: "STRATEGIST", Perspective: "product", Confidence: 0.8},
		},
		Vision: "Build great things",
	}

	result, err := svc.Synthesize(context.Background(), input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if result.Synthesizer != "ORACLE" {
		t.Fatalf("got synthesizer %q, want ORACLE", result.Synthesizer)
	}
	if len(result.Items) != 1 {
		t.Fatalf("got %d items, want 1", len(result.Items))
	}
	if len(result.Dropped) != 1 {
		t.Fatalf("got %d dropped, want 1", len(result.Dropped))
	}
}

func TestSynthesizeExitError(t *testing.T) {
	t.Parallel()

	mock := cruxexec.NewMockRunner()
	mock.Results["opencode"] = &cruxexec.RunResult{
		Stderr:   []byte("model not found"),
		ExitCode: 1,
	}

	svc := &Service{
		Runner:   mock,
		Registry: models.DefaultRegistry(),
		Env:      []string{"HOME=/tmp"},
	}

	_, err := svc.Synthesize(context.Background(), domain.SynthesisInput{})
	if err == nil {
		t.Fatal("expected error for exit code 1")
	}
	if !strings.Contains(err.Error(), "exited 1") {
		t.Fatalf("got %q, want exit error", err)
	}
}

func TestSynthesizeInvalidJSON(t *testing.T) {
	t.Parallel()

	mock := cruxexec.NewMockRunner()
	mock.Results["opencode"] = &cruxexec.RunResult{
		Stdout:   []byte("not json at all"),
		ExitCode: 0,
	}

	svc := &Service{
		Runner:   mock,
		Registry: models.DefaultRegistry(),
		Env:      []string{"HOME=/tmp"},
	}

	_, err := svc.Synthesize(context.Background(), domain.SynthesisInput{})
	if err == nil {
		t.Fatal("expected error for invalid output")
	}
	if !strings.Contains(err.Error(), "no JSON") {
		t.Fatalf("got %q, want no JSON error", err)
	}
}

func TestSynthesizeNoFallback(t *testing.T) {
	t.Parallel()

	// Synthesis has no fallback — single failure should error, not retry other models
	mock := cruxexec.NewMockRunner()
	mock.Errors["opencode"] = &testError{"connection refused"}

	svc := &Service{
		Runner:   mock,
		Registry: models.DefaultRegistry(),
		Env:      []string{"HOME=/tmp"},
	}

	_, err := svc.Synthesize(context.Background(), domain.SynthesisInput{})
	if err == nil {
		t.Fatal("expected error when synthesis fails")
	}
}

func TestRenderSynthesisPrompt(t *testing.T) {
	t.Parallel()

	input := domain.SynthesisInput{
		CouncilOutputs: []domain.CouncilOutput{
			{Councilor: "STRATEGIST", Perspective: "product", Confidence: 0.8},
			{Councilor: "ARCHITECT", Perspective: "engineering", Confidence: 0.9},
		},
		Vision:      "Build the best CLI",
		RepoContext: "10 open issues, 3 PRs",
	}

	prompt := RenderSynthesisPrompt(input)

	checks := []string{
		"# Synthesis Input",
		"## Vision",
		"Build the best CLI",
		"## Repository Context",
		"10 open issues",
		"## Council Outputs",
		"STRATEGIST",
		"ARCHITECT",
	}

	for _, check := range checks {
		if !strings.Contains(prompt, check) {
			t.Errorf("prompt missing %q", check)
		}
	}
}

type testError struct{ msg string }

func (e *testError) Error() string { return e.msg }
