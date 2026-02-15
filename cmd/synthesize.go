package cmd

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/spf13/cobra"

	"github.com/misty-step/crucible/internal/domain"
	"github.com/misty-step/crucible/internal/synthesizer"
	"github.com/misty-step/crucible/internal/telemetry"
)

var (
	synthInputDir string
	synthOutput   string
	synthModel    string
)

var synthesizeCmd = &cobra.Command{
	Use:   "synthesize",
	Short: "Run synthesizer on council outputs",
	RunE:  runSynthesize,
}

func init() {
	synthesizeCmd.Flags().StringVar(&synthInputDir, "input-dir", "", "directory containing council output JSON files (required)")
	synthesizeCmd.Flags().StringVar(&synthOutput, "output", "", "output file path (default: stdout)")
	synthesizeCmd.Flags().StringVar(&synthModel, "model", "", "override synthesis model")
	_ = synthesizeCmd.MarkFlagRequired("input-dir")
	rootCmd.AddCommand(synthesizeCmd)
}

func runSynthesize(cmd *cobra.Command, args []string) error {
	ctx := cmd.Context()

	entries, err := os.ReadDir(synthInputDir)
	if err != nil {
		return fmt.Errorf("read input dir: %w", err)
	}

	var councilOutputs []domain.CouncilOutput
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".json") {
			continue
		}

		data, err := os.ReadFile(filepath.Join(synthInputDir, entry.Name()))
		if err != nil {
			return fmt.Errorf("read %s: %w", entry.Name(), err)
		}

		var co domain.CouncilOutput
		if err := json.Unmarshal(data, &co); err != nil {
			return fmt.Errorf("parse %s: %w", entry.Name(), err)
		}

		councilOutputs = append(councilOutputs, co)
	}

	if len(councilOutputs) == 0 {
		return fmt.Errorf("no council output JSON files found in %s", synthInputDir)
	}

	if verbose {
		fmt.Fprintf(cmd.ErrOrStderr(), "Loaded %d council outputs from %s\n", len(councilOutputs), synthInputDir)
	}

	// Read vision file if available
	var visionText string
	if data, err := os.ReadFile(vision); err == nil {
		visionText = string(data)
	}

	input := domain.SynthesisInput{
		CouncilOutputs: councilOutputs,
		Vision:         visionText,
	}

	synth := &synthesizer.Placeholder{}

	if verbose {
		fmt.Fprintln(cmd.ErrOrStderr(), "Running placeholder synthesizer...")
	}

	synthStart := time.Now()
	result, err := synth.Synthesize(ctx, input)
	synthDuration := time.Since(synthStart)
	if err != nil {
		return fmt.Errorf("synthesize: %w", err)
	}

	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal result: %w", err)
	}

	if synthOutput != "" {
		if err := os.MkdirAll(filepath.Dir(synthOutput), 0o755); err != nil {
			return fmt.Errorf("create output dir: %w", err)
		}
		if err := os.WriteFile(synthOutput, data, 0o644); err != nil {
			return fmt.Errorf("write output: %w", err)
		}
		if verbose {
			fmt.Fprintf(cmd.ErrOrStderr(), "Wrote synthesis result to %s\n", synthOutput)
		}
	} else {
		fmt.Fprintln(cmd.OutOrStdout(), string(data))
	}

	fmt.Fprintf(cmd.ErrOrStderr(), "Synthesis complete: %d items from %d council outputs\n",
		len(result.Items), len(councilOutputs))

	// Generate telemetry from synthesis results
	// Reconstruct spawn results from council outputs (metadata not available post-hoc)
	spawnResults := make([]domain.SpawnResult, len(councilOutputs))
	for i, co := range councilOutputs {
		spawnResults[i] = domain.SpawnResult{
			Output: &co,
		}
	}

	report := telemetry.BuildRunReport(spawnResults, result, synthDuration)
	writer := telemetry.NewWriter(".")
	if err := writer.Write(report); err != nil {
		if verbose {
			fmt.Fprintf(cmd.ErrOrStderr(), "Warning: failed to write telemetry report: %v\n", err)
		}
	} else if verbose {
		fmt.Fprintf(cmd.ErrOrStderr(), "Wrote telemetry report: run_id=%s\n", report.RunID)
	}

	return nil
}
