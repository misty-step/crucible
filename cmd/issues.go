package cmd

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/spf13/cobra"

	"github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
	"github.com/misty-step/crucible/internal/ghissues"
)

var (
	issuesRepo   string
	issuesInput  string
	issuesLabels []string
)

var createIssuesCmd = &cobra.Command{
	Use:   "create-issues",
	Short: "Create GitHub issues from synthesizer output",
	RunE:  runCreateIssues,
}

func init() {
	createIssuesCmd.Flags().StringVar(&issuesRepo, "repo", "", "GitHub repository (owner/name) (required)")
	createIssuesCmd.Flags().StringVar(&issuesInput, "input", "", "path to synthesis result JSON (required)")
	createIssuesCmd.Flags().StringSliceVar(&issuesLabels, "labels", nil, "additional labels to apply to all issues")
	_ = createIssuesCmd.MarkFlagRequired("repo")
	_ = createIssuesCmd.MarkFlagRequired("input")
	rootCmd.AddCommand(createIssuesCmd)
}

func runCreateIssues(cmd *cobra.Command, args []string) error {
	ctx := cmd.Context()

	data, err := os.ReadFile(issuesInput)
	if err != nil {
		return fmt.Errorf("read input: %w", err)
	}

	var result domain.SynthesisResult
	if err := json.Unmarshal(data, &result); err != nil {
		return fmt.Errorf("parse synthesis result: %w", err)
	}

	if err := result.Validate(); err != nil {
		return fmt.Errorf("validate synthesis result: %w", err)
	}

	// Append extra labels if provided
	if len(issuesLabels) > 0 {
		for i := range result.Items {
			result.Items[i].Labels = append(result.Items[i].Labels, issuesLabels...)
		}
	}

	if verbose {
		fmt.Fprintf(cmd.ErrOrStderr(), "Creating %d issues in %s\n", len(result.Items), issuesRepo)
	}

	if dryRun {
		output := ghissues.FormatDryRun(result.Items, ghissues.DefaultMilestones())
		fmt.Fprint(cmd.OutOrStdout(), output)
		return nil
	}

	creator := &ghissues.Creator{
		Runner:     &cruxexec.OSRunner{},
		Milestones: ghissues.DefaultMilestones(),
		Repo:       issuesRepo,
	}

	created, err := creator.Create(ctx, result.Items)
	if err != nil {
		// Print any partially created issues before returning error
		for _, c := range created {
			fmt.Fprintf(cmd.OutOrStdout(), "Created: %s (%s)\n", c.Title, c.URL)
		}
		return fmt.Errorf("create issues: %w", err)
	}

	for _, c := range created {
		fmt.Fprintf(cmd.OutOrStdout(), "Created #%d: %s (%s)\n", c.Number, c.Title, c.URL)
	}

	fmt.Fprintf(cmd.OutOrStdout(), "\n%d issues created in %s\n", len(created), issuesRepo)
	return nil
}
