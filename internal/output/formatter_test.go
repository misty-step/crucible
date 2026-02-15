package output

import (
	"strings"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
)

func TestFormat_Valid(t *testing.T) {
	tests := []struct {
		name   string
		format Format
		want   bool
	}{
		{"github is valid", GitHub, true},
		{"markdown is valid", Markdown, true},
		{"json is valid", JSON, true},
		{"stdout is valid", Stdout, true},
		{"empty is invalid", "", false},
		{"unknown is invalid", "unknown", false},
		{"csv is invalid", "csv", false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := tt.format.Valid()
			if got != tt.want {
				t.Errorf("Valid() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestFormatter_FormatResult(t *testing.T) {
	formatter := NewFormatter()
	result := sampleSynthesisResult()

	t.Run("github format returns error", func(t *testing.T) {
		_, err := formatter.FormatResult(result, GitHub)
		if err == nil {
			t.Error("expected error for github format")
		}
	})

	t.Run("markdown format returns markdown", func(t *testing.T) {
		output, err := formatter.FormatResult(result, Markdown)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(output, "# Groomed Backlog") {
			t.Error("expected markdown header")
		}
		if !strings.Contains(output, result.Items[0].Title) {
			t.Error("expected item title in output")
		}
	})

	t.Run("json format returns json", func(t *testing.T) {
		output, err := formatter.FormatResult(result, JSON)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(output, `"synthesizer"`) {
			t.Error("expected synthesizer field in JSON")
		}
		if !strings.Contains(output, result.Synthesizer) {
			t.Error("expected synthesizer value in JSON")
		}
	})

	t.Run("stdout format returns summary", func(t *testing.T) {
		output, err := formatter.FormatResult(result, Stdout)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(output, result.Summary) {
			t.Error("expected summary in output")
		}
		if !strings.Contains(output, "2 items generated") {
			t.Error("expected item count in output")
		}
	})

	t.Run("unknown format returns error", func(t *testing.T) {
		_, err := formatter.FormatResult(result, Format("unknown"))
		if err == nil {
			t.Error("expected error for unknown format")
		}
	})
}

func TestFormatter_FormatMarkdown(t *testing.T) {
	formatter := NewFormatter()

	t.Run("empty result", func(t *testing.T) {
		result := &domain.SynthesisResult{
			Summary: "Test summary",
			Items:   nil,
		}
		output := formatter.FormatMarkdown(result)
		if !strings.Contains(output, "No backlog items generated") {
			t.Error("expected 'no items' message")
		}
	})

	t.Run("result with items", func(t *testing.T) {
		result := sampleSynthesisResult()
		output := formatter.FormatMarkdown(result)

		// Check main sections
		if !strings.Contains(output, "# Groomed Backlog: Test Result") {
			t.Error("expected title")
		}

		// Check item content
		if !strings.Contains(output, "Fix critical bug") {
			t.Error("expected first item title")
		}
		if !strings.Contains(output, "Add new feature") {
			t.Error("expected second item title")
		}

		// Check metadata
		if !strings.Contains(output, "**Priority:** p0") {
			t.Error("expected priority metadata")
		}
		if !strings.Contains(output, "**Type:** bug") {
			t.Error("expected type metadata")
		}
		if !strings.Contains(output, "**Horizon:** now") {
			t.Error("expected horizon metadata")
		}
	})

	t.Run("result with conflicts", func(t *testing.T) {
		result := sampleSynthesisResult()
		result.Conflicts = []domain.Conflict{
			{
				Item:         "Some Item",
				Disagreement: "Council disagreement",
				Resolution:   "Compromise reached",
			},
		}
		output := formatter.FormatMarkdown(result)
		if !strings.Contains(output, "## Conflicts Resolved") {
			t.Error("expected conflicts section")
		}
	})

	t.Run("result with dropped items", func(t *testing.T) {
		result := sampleSynthesisResult()
		result.Dropped = []domain.DroppedItem{
			{
				Title:  "Old Idea",
				Reason: "Out of scope",
			},
		}
		output := formatter.FormatMarkdown(result)
		if !strings.Contains(output, "## Dropped Items") {
			t.Error("expected dropped section")
		}
	})
}

func TestFormatter_FormatJSON(t *testing.T) {
	formatter := NewFormatter()
	result := sampleSynthesisResult()

	t.Run("valid result returns JSON", func(t *testing.T) {
		output, err := formatter.FormatJSON(result)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		// Verify it's valid JSON by checking for key features
		if !strings.HasPrefix(output, "{") {
			t.Error("expected JSON object start")
		}
		if !strings.Contains(output, `"summary": "Test Result"`) {
			t.Error("expected summary field")
		}
	})
}

func TestFormatter_FormatStdout(t *testing.T) {
	formatter := NewFormatter()

	t.Run("empty result", func(t *testing.T) {
		result := &domain.SynthesisResult{
			Summary: "Empty",
			Items:   []domain.SynthesisItem{},
		}
		output := formatter.FormatStdout(result)
		if !strings.Contains(output, "0 items generated") {
			t.Error("expected zero count")
		}
	})

	t.Run("result grouped by priority", func(t *testing.T) {
		result := sampleSynthesisResult()
		output := formatter.FormatStdout(result)

		if !strings.Contains(output, "P0 - Critical") {
			t.Error("expected P0 section")
		}
		if !strings.Contains(output, "Fix critical bug") {
			t.Error("expected bug item")
		}
		if !strings.Contains(output, "[now]") {
			t.Error("expected horizon in brackets")
		}
	})

	t.Run("includes conflict and drop counts", func(t *testing.T) {
		result := sampleSynthesisResult()
		result.Conflicts = []domain.Conflict{{Item: "X", Disagreement: "", Resolution: ""}}
		result.Dropped = []domain.DroppedItem{{Title: "Y", Reason: ""}}
		output := formatter.FormatStdout(result)

		if !strings.Contains(output, "Conflicts resolved: 1") {
			t.Error("expected conflict count")
		}
		if !strings.Contains(output, "Items dropped: 1") {
			t.Error("expected dropped count")
		}
	})
}

func sampleSynthesisResult() *domain.SynthesisResult {
	return &domain.SynthesisResult{
		Synthesizer: "test-synthesizer",
		Model:       "test-model",
		Summary:     "Test Result",
		Items: []domain.SynthesisItem{
			{
				Title:    "Fix critical bug",
				Priority: domain.P0,
				Type:     domain.Bug,
				Horizon:  domain.Now,
				Effort:   domain.Small,
				Body:     "This is a critical bug that needs fixing immediately.",
				Labels:   []string{"security"},
				CouncilSupport: domain.CouncilSupport{
					ProposedBy: []string{"critic", "architect"},
					Consensus:  domain.Strong,
				},
			},
			{
				Title:    "Add new feature",
				Priority: domain.P2,
				Type:     domain.Feature,
				Horizon:  domain.Next,
				Effort:   domain.Medium,
				Body:     "A nice new feature that would be helpful.",
				Labels:   []string{"enhancement"},
				CouncilSupport: domain.CouncilSupport{
					ProposedBy: []string{"architect"},
					Consensus:  domain.Moderate,
				},
			},
		},
	}
}
