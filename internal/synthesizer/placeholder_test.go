package synthesizer

import (
	"context"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
)

func TestPlaceholderSynthesizeBasic(t *testing.T) {
	t.Parallel()

	input := domain.SynthesisInput{
		CouncilOutputs: []domain.CouncilOutput{
			{
				Councilor:   "STRATEGIST",
				Perspective: "product",
				Confidence:  0.85,
				Items: []domain.CouncilItem{
					{Title: "Add auth", Priority: domain.P0, Type: domain.Feature, Effort: domain.Large, Rationale: "Need auth", Risk: "Blocks launch"},
					{Title: "Add logging", Priority: domain.P2, Type: domain.Task, Effort: domain.Small, Rationale: "Observability", Risk: "None"},
				},
			},
		},
	}

	synth := &Placeholder{}
	result, err := synth.Synthesize(context.Background(), input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if result.Synthesizer != "PLACEHOLDER" {
		t.Errorf("got synthesizer %q, want PLACEHOLDER", result.Synthesizer)
	}
	if len(result.Items) != 2 {
		t.Fatalf("got %d items, want 2", len(result.Items))
	}

	// Items should be sorted by priority (p0 first)
	if result.Items[0].Priority != domain.P0 {
		t.Errorf("first item priority = %q, want p0", result.Items[0].Priority)
	}
	if result.Items[1].Priority != domain.P2 {
		t.Errorf("second item priority = %q, want p2", result.Items[1].Priority)
	}
}

func TestPlaceholderSynthesizeDeduplication(t *testing.T) {
	t.Parallel()

	input := domain.SynthesisInput{
		CouncilOutputs: []domain.CouncilOutput{
			{
				Councilor:   "STRATEGIST",
				Perspective: "product",
				Items: []domain.CouncilItem{
					{Title: "Add auth", Priority: domain.P1, Type: domain.Feature, Effort: domain.Large, Rationale: "Need auth", Risk: "Risk"},
				},
			},
			{
				Councilor:   "ARCHITECT",
				Perspective: "engineering",
				Items: []domain.CouncilItem{
					{Title: "Add auth", Priority: domain.P0, Type: domain.Feature, Effort: domain.Large, Rationale: "Must have", Risk: "Risk"},
					{Title: "Add tests", Priority: domain.P1, Type: domain.Task, Effort: domain.Medium, Rationale: "Quality", Risk: "Risk"},
				},
			},
		},
	}

	synth := &Placeholder{}
	result, err := synth.Synthesize(context.Background(), input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(result.Items) != 2 {
		t.Fatalf("got %d items, want 2 (deduplication)", len(result.Items))
	}

	// "Add auth" should take the higher priority (p0)
	for _, item := range result.Items {
		if item.Title == "Add auth" {
			if item.Priority != domain.P0 {
				t.Errorf("deduped item priority = %q, want p0", item.Priority)
			}
			if len(item.CouncilSupport.ProposedBy) != 2 {
				t.Errorf("got %d supporters, want 2", len(item.CouncilSupport.ProposedBy))
			}
			if item.CouncilSupport.Consensus != domain.Strong {
				t.Errorf("got consensus %q, want strong (2/2)", item.CouncilSupport.Consensus)
			}
		}
	}
}

func TestPlaceholderSynthesizeEmpty(t *testing.T) {
	t.Parallel()

	synth := &Placeholder{}
	_, err := synth.Synthesize(context.Background(), domain.SynthesisInput{})
	if err == nil {
		t.Fatal("expected error for empty council outputs")
	}
}

func TestPlaceholderHorizonMapping(t *testing.T) {
	t.Parallel()

	input := domain.SynthesisInput{
		CouncilOutputs: []domain.CouncilOutput{
			{
				Councilor:   "TEST",
				Perspective: "product",
				Items: []domain.CouncilItem{
					{Title: "P0 item", Priority: domain.P0, Type: domain.Feature, Effort: domain.Large, Rationale: "R", Risk: "R"},
					{Title: "P1 item", Priority: domain.P1, Type: domain.Feature, Effort: domain.Medium, Rationale: "R", Risk: "R"},
					{Title: "P2 item", Priority: domain.P2, Type: domain.Task, Effort: domain.Small, Rationale: "R", Risk: "R"},
					{Title: "P3 item", Priority: domain.P3, Type: domain.Task, Effort: domain.Small, Rationale: "R", Risk: "R"},
				},
			},
		},
	}

	synth := &Placeholder{}
	result, err := synth.Synthesize(context.Background(), input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	horizonMap := make(map[string]domain.Horizon)
	for _, item := range result.Items {
		horizonMap[item.Title] = item.Horizon
	}

	if horizonMap["P0 item"] != domain.Now {
		t.Errorf("P0 horizon = %q, want now", horizonMap["P0 item"])
	}
	if horizonMap["P1 item"] != domain.Next {
		t.Errorf("P1 horizon = %q, want next", horizonMap["P1 item"])
	}
	if horizonMap["P2 item"] != domain.Later {
		t.Errorf("P2 horizon = %q, want later", horizonMap["P2 item"])
	}
	if horizonMap["P3 item"] != domain.Later {
		t.Errorf("P3 horizon = %q, want later", horizonMap["P3 item"])
	}
}
