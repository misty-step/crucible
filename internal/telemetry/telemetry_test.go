package telemetry

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/misty-step/crucible/internal/domain"
)

func TestWriter_Write(t *testing.T) {
	tmpDir := t.TempDir()
	writer := NewWriter(tmpDir)

	report := RunReport{
		RunID:     "test-run-123",
		Timestamp: time.Now().UTC().Format(time.RFC3339),
		Perspectives: map[string]PerspectiveStats{
			"product": {
				Model:      "gpt-4",
				Retries:    0,
				DurationMs: 12340,
				Items:      5,
				Skipped:    false,
			},
		},
		Synthesis: SynthesisStats{
			DurationMs: 23450,
			ItemsIn:    22,
			ItemsOut:   15,
			Conflicts:  3,
			Dropped:    7,
		},
	}

	err := writer.Write(report)
	if err != nil {
		t.Fatalf("Write failed: %v", err)
	}

	// Verify file was created
	expectedPath := filepath.Join(tmpDir, ".crucible", "runs", "run_test-run-123.json")
	data, err := os.ReadFile(expectedPath)
	if err != nil {
		t.Fatalf("Failed to read written file: %v", err)
	}

	var readReport RunReport
	if err := json.Unmarshal(data, &readReport); err != nil {
		t.Fatalf("Failed to unmarshal report: %v", err)
	}

	if readReport.RunID != report.RunID {
		t.Errorf("RunID mismatch: got %q, want %q", readReport.RunID, report.RunID)
	}

	if len(readReport.Perspectives) != 1 {
		t.Errorf("Perspectives count: got %d, want 1", len(readReport.Perspectives))
	}

	if readReport.Synthesis.ItemsOut != 15 {
		t.Errorf("Synthesis ItemsOut: got %d, want 15", readReport.Synthesis.ItemsOut)
	}
}

func TestBuildRunReport(t *testing.T) {
	councilResults := []domain.SpawnResult{
		{
			Output: &domain.CouncilOutput{
				Perspective: "product",
				Items: []domain.CouncilItem{
					{Title: "Item 1"},
					{Title: "Item 2"},
				},
			},
			Model:    "gpt-4",
			Retries:  0,
			Duration: 12340 * time.Millisecond,
		},
		{
			Output: &domain.CouncilOutput{
				Perspective: "engineering",
				Items: []domain.CouncilItem{
					{Title: "Item 3"},
					{Title: "Item 4"},
					{Title: "Item 5"},
				},
			},
			Model:    "claude-3",
			Retries:  1,
			Duration: 15670 * time.Millisecond,
		},
	}

	synthesisResult := &domain.SynthesisResult{
		Items: []domain.SynthesisItem{
			{Title: "Final 1"},
			{Title: "Final 2"},
		},
		Conflicts: []domain.Conflict{
			{Item: "conflict1"},
			{Item: "conflict2"},
		},
		Dropped: []domain.DroppedItem{
			{Title: "dropped1"},
			{Title: "dropped2"},
			{Title: "dropped3"},
		},
	}

	synthesisDuration := 23450 * time.Millisecond

	report := BuildRunReport(councilResults, synthesisResult, synthesisDuration)

	// Verify run_id is generated
	if report.RunID == "" {
		t.Error("RunID should be generated")
	}

	// Verify timestamp is set
	if report.Timestamp == "" {
		t.Error("Timestamp should be set")
	}

	// Verify perspectives
	if len(report.Perspectives) != 2 {
		t.Errorf("Expected 2 perspectives, got %d", len(report.Perspectives))
	}

	productStats, ok := report.Perspectives["product"]
	if !ok {
		t.Error("Missing product perspective stats")
	} else {
		if productStats.Model != "gpt-4" {
			t.Errorf("Product model: got %q, want gpt-4", productStats.Model)
		}
		if productStats.Items != 2 {
			t.Errorf("Product items: got %d, want 2", productStats.Items)
		}
		if productStats.DurationMs != 12340 {
			t.Errorf("Product duration: got %d, want 12340", productStats.DurationMs)
		}
	}

	engStats, ok := report.Perspectives["engineering"]
	if !ok {
		t.Error("Missing engineering perspective stats")
	} else {
		if engStats.Model != "claude-3" {
			t.Errorf("Engineering model: got %q, want claude-3", engStats.Model)
		}
		if engStats.Items != 3 {
			t.Errorf("Engineering items: got %d, want 3", engStats.Items)
		}
		if engStats.Retries != 1 {
			t.Errorf("Engineering retries: got %d, want 1", engStats.Retries)
		}
	}

	// Verify synthesis stats
	if report.Synthesis.ItemsIn != 5 {
		t.Errorf("Synthesis ItemsIn: got %d, want 5", report.Synthesis.ItemsIn)
	}
	if report.Synthesis.ItemsOut != 2 {
		t.Errorf("Synthesis ItemsOut: got %d, want 2", report.Synthesis.ItemsOut)
	}
	if report.Synthesis.Conflicts != 2 {
		t.Errorf("Synthesis Conflicts: got %d, want 2", report.Synthesis.Conflicts)
	}
	if report.Synthesis.Dropped != 3 {
		t.Errorf("Synthesis Dropped: got %d, want 3", report.Synthesis.Dropped)
	}
	if report.Synthesis.DurationMs != 23450 {
		t.Errorf("Synthesis DurationMs: got %d, want 23450", report.Synthesis.DurationMs)
	}
}

func TestBuildRunReport_SkippedPerspectives(t *testing.T) {
	councilResults := []domain.SpawnResult{
		{
			Output: &domain.CouncilOutput{
				Perspective: "product",
				Items:       []domain.CouncilItem{{Title: "Item 1"}},
			},
			Model:    "gpt-4",
			Duration: 1000 * time.Millisecond,
		},
		{
			Output:  nil,
			Skipped: true,
			Error:   nil,
		},
	}

	report := BuildRunReport(councilResults, nil, 0)

	// Should only have the non-skipped perspective
	if len(report.Perspectives) != 1 {
		t.Errorf("Expected 1 perspective, got %d", len(report.Perspectives))
	}
}

func TestBuildRunReport_NoSynthesis(t *testing.T) {
	councilResults := []domain.SpawnResult{
		{
			Output: &domain.CouncilOutput{
				Perspective: "product",
				Items:       []domain.CouncilItem{{Title: "Item 1"}},
			},
			Model:    "gpt-4",
			Duration: 1000 * time.Millisecond,
		},
	}

	report := BuildRunReport(councilResults, nil, 0)

	// Synthesis stats should be zero values
	if report.Synthesis.DurationMs != 0 {
		t.Errorf("Synthesis DurationMs should be 0 when no synthesis, got %d", report.Synthesis.DurationMs)
	}
}
