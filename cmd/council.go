package cmd

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/spf13/cobra"

	"github.com/misty-step/crucible/internal/council"
	"github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
	"github.com/misty-step/crucible/internal/models"
	"github.com/misty-step/crucible/internal/reposcanner"
)

var (
	councilRepo      string
	councilOutputDir string
	councilModel     string
)

var councilCmd = &cobra.Command{
	Use:   "council",
	Short: "Run multi-model council for backlog grooming",
	RunE:  runCouncil,
}

func init() {
	councilCmd.Flags().StringVar(&councilRepo, "repo", "", "repository path (required)")
	councilCmd.Flags().StringVar(&councilOutputDir, "output-dir", "./crucible-output/council/", "directory for council output JSON files")
	councilCmd.Flags().StringVar(&councilModel, "model", "", "override model for all perspectives")
	_ = councilCmd.MarkFlagRequired("repo")
	rootCmd.AddCommand(councilCmd)
}

func runCouncil(cmd *cobra.Command, args []string) error {
	ctx := cmd.Context()

	runner := &cruxexec.OSRunner{}
	registry := models.DefaultRegistry()

	gatherer := &reposcanner.CLIGatherer{
		Runner:     runner,
		VisionPath: vision,
		Dir:        councilRepo,
	}

	if verbose {
		fmt.Fprintln(cmd.ErrOrStderr(), "Gathering repository context...")
	}

	repoCtx, err := gatherer.Gather(ctx)
	if err != nil {
		return fmt.Errorf("gather repo context: %w", err)
	}

	input := domain.CouncilInput{
		Vision:    repoCtx.Vision,
		RepoState: repoCtx.RepoState,
		Date:      time.Now().Format("2006-01-02"),
	}

	if verbose {
		fmt.Fprintf(cmd.ErrOrStderr(), "Council input: %d commits, %d issues, %d PRs\n",
			len(input.RepoState.RecentCommits),
			len(input.RepoState.OpenIssues),
			len(input.RepoState.OpenPRs))
	}

	if dryRun {
		fmt.Fprintln(cmd.OutOrStdout(), "dry-run: would run council with gathered context")
		return nil
	}

	spawner := council.NewSpawner(runner, registry)

	if verbose {
		fmt.Fprintln(cmd.ErrOrStderr(), "Running council perspectives...")
	}

	results, err := spawner.RunCouncil(ctx, input)
	if err != nil {
		return fmt.Errorf("council run: %w", err)
	}

	if err := os.MkdirAll(councilOutputDir, 0o755); err != nil {
		return fmt.Errorf("create output dir: %w", err)
	}

	written := 0
	for i, r := range results {
		if r.Output == nil {
			if verbose {
				fmt.Fprintf(cmd.ErrOrStderr(), "Perspective %d skipped: %v\n", i, r.Error)
			}
			continue
		}

		filename := fmt.Sprintf("council_%s.json", r.Output.Perspective)
		outPath := filepath.Join(councilOutputDir, filename)

		data, err := json.MarshalIndent(r.Output, "", "  ")
		if err != nil {
			return fmt.Errorf("marshal output for %s: %w", r.Output.Perspective, err)
		}

		if err := os.WriteFile(outPath, data, 0o644); err != nil {
			return fmt.Errorf("write %s: %w", outPath, err)
		}

		written++
		if verbose {
			fmt.Fprintf(cmd.ErrOrStderr(), "Wrote %s (model=%s, retries=%d, duration=%s)\n",
				outPath, r.Model, r.Retries, r.Duration)
		}
	}

	fmt.Fprintf(cmd.OutOrStdout(), "Council complete: %d/%d perspectives written to %s\n",
		written, len(results), councilOutputDir)

	return nil
}
