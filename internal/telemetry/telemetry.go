// Package telemetry provides quality reporting for council runs.
package telemetry

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/google/uuid"
	"github.com/misty-step/crucible/internal/domain"
)

// RunReport captures quality metrics for a complete council run.
type RunReport struct {
	RunID     string                 `json:"run_id"`
	Timestamp string                 `json:"timestamp"`
	Perspectives map[string]PerspectiveStats `json:"perspectives"`
	Synthesis    SynthesisStats             `json:"synthesis"`
}

// PerspectiveStats tracks per-agent metrics.
type PerspectiveStats struct {
	Model    string `json:"model"`
	Retries  int    `json:"retries"`
	DurationMs int64 `json:"duration_ms"`
	Items    int    `json:"items"`
	Skipped  bool   `json:"skipped"`
}

// SynthesisStats tracks synthesizer metrics.
type SynthesisStats struct {
	DurationMs int64 `json:"duration_ms"`
	ItemsIn    int   `json:"items_in"`
	ItemsOut   int   `json:"items_out"`
	Conflicts  int   `json:"conflicts"`
	Dropped    int   `json:"dropped"`
}

// Writer persists telemetry reports to disk.
type Writer struct {
	BaseDir string
}

// NewWriter creates a telemetry writer.
func NewWriter(baseDir string) *Writer {
	return &Writer{BaseDir: baseDir}
}

// ensureDir creates the telemetry directory if needed.
func (w *Writer) ensureDir() error {
	runsDir := filepath.Join(w.BaseDir, ".crucible", "runs")
	return os.MkdirAll(runsDir, 0755)
}

// Write persists a run report to disk.
func (w *Writer) Write(report RunReport) error {
	if err := w.ensureDir(); err != nil {
		return fmt.Errorf("create runs dir: %w", err)
	}

	data, err := json.MarshalIndent(report, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal report: %w", err)
	}

	filename := fmt.Sprintf("run_%s.json", report.RunID)
	path := filepath.Join(w.BaseDir, ".crucible", "runs", filename)

	if err := os.WriteFile(path, data, 0644); err != nil {
		return fmt.Errorf("write report: %w", err)
	}

	return nil
}

// BuildRunReport creates a telemetry report from council and synthesis results.
func BuildRunReport(
	councilResults []domain.SpawnResult,
	synthesisResult *domain.SynthesisResult,
	synthesisDuration time.Duration,
) RunReport {
	report := RunReport{
		RunID:        uuid.New().String(),
		Timestamp:    time.Now().UTC().Format(time.RFC3339),
		Perspectives: make(map[string]PerspectiveStats),
	}

	for _, r := range councilResults {
		if r.Output == nil {
			continue
		}

		stats := PerspectiveStats{
			Model:      r.Model,
			Retries:    r.Retries,
			DurationMs: r.Duration.Milliseconds(),
			Skipped:    r.Skipped,
		}

		if r.Output != nil {
			stats.Items = len(r.Output.Items)
		}

		report.Perspectives[r.Output.Perspective] = stats
	}

	if synthesisResult != nil {
		report.Synthesis = SynthesisStats{
			DurationMs: synthesisDuration.Milliseconds(),
			ItemsIn:    countItemsIn(councilResults),
			ItemsOut:   len(synthesisResult.Items),
			Conflicts:  len(synthesisResult.Conflicts),
			Dropped:    len(synthesisResult.Dropped),
		}
	}

	return report
}

func countItemsIn(results []domain.SpawnResult) int {
	total := 0
	for _, r := range results {
		if r.Output != nil {
			total += len(r.Output.Items)
		}
	}
	return total
}
