package cmd

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/spf13/cobra"

	"github.com/misty-step/crucible/internal/council"
	"github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
	"github.com/misty-step/crucible/internal/models"
	"github.com/misty-step/crucible/internal/reposcanner"
)

var (
	councilRepo        string
	councilOutputDir   string
	councilModel       string
	councilInteractive bool
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
	councilCmd.Flags().BoolVar(&councilInteractive, "interactive", false, "prompt for human input before running council")
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

	if councilInteractive {
		if err := promptForHumanInput(cmd, repoCtx, &input); err != nil {
			return fmt.Errorf("read human input: %w", err)
		}
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

// promptForHumanInput prints a repo context summary and reads multiline input from the user.
func promptForHumanInput(cmd *cobra.Command, repoCtx *reposcanner.RepoContext, input *domain.CouncilInput) error {
	out := cmd.OutOrStdout()
	in := cmd.InOrStdin()

	fmt.Fprintln(out, "\n=== Repository Context Summary ===")
	if repoCtx.Vision != "" {
		fmt.Fprintln(out, "\nVision:")
		fmt.Fprintln(out, repoCtx.Vision)
	}

	state := repoCtx.RepoState
	if len(state.RecentCommits) > 0 {
		fmt.Fprintf(out, "\nRecent Commits: %d\n", len(state.RecentCommits))
		for i, c := range state.RecentCommits {
			if i >= 5 {
				fmt.Fprintln(out, "  ...")
				break
			}
			fmt.Fprintf(out, "  - %s\n", c)
		}
	}

	if len(state.OpenIssues) > 0 {
		fmt.Fprintf(out, "\nOpen Issues: %d\n", len(state.OpenIssues))
	}

	if len(state.OpenPRs) > 0 {
		fmt.Fprintf(out, "Open PRs: %d\n", len(state.OpenPRs))
	}

	fmt.Fprintln(out, "\n=== Human Input ===")
	fmt.Fprintln(out, "What priorities, concerns, or ideas should the council consider?")
	fmt.Fprintln(out, "(Enter multiple lines, end with an empty line or Ctrl+D):")

	scanner := bufio.NewScanner(in)
	var lines []string

	for scanner.Scan() {
		line := scanner.Text()
		if line == "" {
			break
		}
		lines = append(lines, line)
	}

	if err := scanner.Err(); err != nil {
		return err
	}

	input.HumanInput = strings.Join(lines, "\n")
	return nil
}
