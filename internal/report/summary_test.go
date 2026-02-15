package report

import (
	"bytes"
	"strings"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
)

func TestPrintSynthesisSummary(t *testing.T) {
	t.Parallel()

	result := &domain.SynthesisResult{
		Synthesizer: "ORACLE",
		Model:       "test-model",
		Summary:     "Test synthesis",
		Items: []domain.SynthesisItem{
			{
				Title:      "High confidence item",
				Priority:   domain.P0,
				Type:       domain.Feature,
				Horizon:    domain.Now,
				Effort:     domain.Medium,
				Body:       "Test body",
				Confidence: 0.85,
				CouncilSupport: domain.CouncilSupport{
					ProposedBy: []string{"STRATEGIST"},
					Consensus:  domain.Strong,
				},
			},
			{
				Title:      "Borderline item",
				Priority:   domain.P1,
				Type:       domain.Task,
				Horizon:    domain.Next,
				Effort:     domain.Small,
				Body:       "Borderline body",
				Confidence: 0.52,
				CouncilSupport: domain.CouncilSupport{
					ProposedBy: []string{"ARCHITECT", "MERCHANT"},
					Consensus:  domain.Moderate,
				},
			},
		},
		Dropped: []domain.DroppedItem{
			{
				Title:          "Low confidence dropped",
				Reason:         "Not aligned with vision",
				Confidence:     0.25,
				CouncilSupport: []string{"MERCHANT"},
			},
			{
				Title:          "Borderline dropped",
				Reason:         "Duplicate of existing issue",
				Confidence:     0.45,
				CouncilSupport: []string{"STRATEGIST"},
			},
		},
	}

	var buf bytes.Buffer
	reporter := NewSummaryReporter(&buf)
	reporter.PrintSynthesisSummary(result)

	output := buf.String()

	// Check sections are present
	if !strings.Contains(output, "DROPPED (2 items)") {
		t.Errorf("expected DROPPED section with 2 items, got:\n%s", output)
	}
	if !strings.Contains(output, "BORDERLINE (1 items)") {
		t.Errorf("expected BORDERLINE section with 1 item, got:\n%s", output)
	}
	if !strings.Contains(output, "ACCEPTED (1 items)") {
		t.Errorf("expected ACCEPTED section with 1 item, got:\n%s", output)
	}

	// Check confidence values are displayed
	if !strings.Contains(output, "confidence: 0.85") {
		t.Errorf("expected confidence 0.85, got:\n%s", output)
	}
	if !strings.Contains(output, "confidence: 0.52") {
		t.Errorf("expected confidence 0.52, got:\n%s", output)
	}
	if !strings.Contains(output, "confidence: 0.25") {
		t.Errorf("expected confidence 0.25, got:\n%s", output)
	}

	// Check symbols for dropped items
	if !strings.Contains(output, "✗") {
		t.Errorf("expected ✗ symbol for low confidence, got:\n%s", output)
	}
	if !strings.Contains(output, "⚠") {
		t.Errorf("expected ⚠ symbol for medium-low confidence, got:\n%s", output)
	}
	if !strings.Contains(output, "✓") {
		t.Errorf("expected ✓ symbol for accepted, got:\n%s", output)
	}
}

func TestPrintSynthesisSummaryEmpty(t *testing.T) {
	t.Parallel()

	result := &domain.SynthesisResult{
		Synthesizer: "ORACLE",
		Summary:     "Empty test",
		Items:       []domain.SynthesisItem{},
		Dropped:     []domain.DroppedItem{},
	}

	var buf bytes.Buffer
	reporter := NewSummaryReporter(&buf)
	reporter.PrintSynthesisSummary(result)

	output := buf.String()

	// Should still print header
	if !strings.Contains(output, "SYNTHESIS SUMMARY") {
		t.Errorf("expected summary header, got:\n%s", output)
	}

	// Should not have DROPPED or BORDERLINE sections
	if strings.Contains(output, "DROPPED") {
		t.Errorf("should not have DROPPED section for empty results, got:\n%s", output)
	}
	if strings.Contains(output, "BORDERLINE") {
		t.Errorf("should not have BORDERLINE section for empty results, got:\n%s", output)
	}
}

func TestPrintDroppedItemsSorting(t *testing.T) {
	t.Parallel()

	dropped := []domain.DroppedItem{
		{Title: "High", Reason: "test", Confidence: 0.8},
		{Title: "Low", Reason: "test", Confidence: 0.2},
		{Title: "Medium", Reason: "test", Confidence: 0.5},
	}

	result := &domain.SynthesisResult{
		Summary: "Sorting test",
		Dropped: dropped,
	}

	var buf bytes.Buffer
	reporter := NewSummaryReporter(&buf)
	reporter.PrintSynthesisSummary(result)

	output := buf.String()

	// Check that items are sorted by confidence (lowest first)
	lowIdx := strings.Index(output, "Low")
	mediumIdx := strings.Index(output, "Medium")
	highIdx := strings.Index(output, "High")

	if lowIdx == -1 || mediumIdx == -1 || highIdx == -1 {
		t.Fatalf("expected all items in output, got:\n%s", output)
	}

	if !(lowIdx < mediumIdx && mediumIdx < highIdx) {
		t.Errorf("expected ascending order by confidence, got low@%d, medium@%d, high@%d\n%s",
			lowIdx, mediumIdx, highIdx, output)
	}
}

func TestSymbolForConfidence(t *testing.T) {
	t.Parallel()

	reporter := NewSummaryReporter(&bytes.Buffer{})

	tests := []struct {
		confidence float64
		want       string
	}{
		{0.0, "✗"},
		{0.1, "✗"},
		{0.29, "✗"},
		{0.3, "⚠"},
		{0.4, "⚠"},
		{0.49, "⚠"},
		{0.5, "~"},
		{0.6, "~"},
		{0.7, "~"},
		{1.0, "~"},
	}

	for _, tt := range tests {
		got := reporter.symbolForConfidence(tt.confidence)
		if got != tt.want {
			t.Errorf("symbolForConfidence(%.2f) = %q, want %q", tt.confidence, got, tt.want)
		}
	}
}
