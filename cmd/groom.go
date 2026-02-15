package cmd

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/spf13/cobra"

	"github.com/misty-step/crucible/internal/council"
	"github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
	"github.com/misty-step/crucible/internal/ghissues"
	"github.com/misty-step/crucible/internal/models"
	"github.com/misty-step/crucible/internal/output"
	"github.com/misty-step/crucible/internal/reposcanner"
	"github.com/misty-step/crucible/internal/synthesizer"
)

var (
	groomRepo      string
	groomOutput    string
	groomModel     string
	groomOutputDir string
	groomLabels    []string
)

var groomCmd = &cobra.Command{
	Use:   "groom",
	Short: "Run full backlog grooming pipeline",
	Long:  `Groom runs council, synthesizes outputs, and emits results in your chosen format.`,
	RunE:  runGroom,
}

func init() {
	groomCmd.Flags().StringVar(&groomRepo, "repo", ".", "repository path (default: current directory)")
	groomCmd.Flags().StringVar(&groomOutput, "output", "github", "output format: github, markdown, json, stdout")
	groomCmd.Flags().StringVar(&groomModel, "model", "", "override model for all perspectives")
	groomCmd.Flags().StringVar(&groomOutputDir, "output-dir", "./crucible-output", "directory for output files")
	groomCmd.Flags().StringSliceVar(&groomLabels, "labels", nil, "additional labels to apply to all issues (github format)")
	rootCmd.AddCommand(groomCmd)
}

func runGroom(cmd *cobra.Command, _ []string) error {
	ctx := cmd.Context()

	// Validate output format
	format := output.Format(groomOutput)
	if !format.Valid() {
		return fmt.Errorf("invalid output format %q: must be one of: github, markdown, json, stdout", groomOutput)
	}

	if verbose {
		fmt.Fprintf(cmd.ErrOrStderr(), "Grooming repository: %s\n", groomRepo)
		fmt.Fprintf(cmd.ErrOrStderr(), "Output format: %s\n", groomOutput)
	}

	// Step 1: Gather repository context
	repoCtx, err := gatherRepoContext(ctx)
	if err != nil {
		return err
	}

	// Step 2: Run council
	councilOutputs, err := runCouncilStep(ctx, repoCtx)
	if err != nil {
		return err
	}

	if len(councilOutputs) == 0 {
		return fmt.Errorf("no council outputs generated")
	}

	// Step 3: Synthesize
	result, err := runSynthesisStep(ctx, councilOutputs, repoCtx.Vision)
	if err != nil {
		return err
	}

	// Step 4: Emit results
	return emitResults(ctx, cmd, result, format)
}

func gatherRepoContext(ctx context.Context) (*reposcanner.RepoContext, error) {
	runner := &cruxexec.OSRunner{}
	gatherer := &reposcanner.CLIGatherer{
		Runner:     runner,
		VisionPath: vision,
		Dir:        groomRepo,
	}

	if verbose {
		fmt.Fprintln(os.Stderr, "Gathering repository context...")
	}

	repoCtx, err := gatherer.Gather(ctx)
	if err != nil {
		return nil, fmt.Errorf("gather repo context: %w", err)
	}

	if verbose {
		fmt.Fprintf(os.Stderr, "Found: %d commits, %d issues, %d PRs\n",
			len(repoCtx.RepoState.RecentCommits),
			len(repoCtx.RepoState.OpenIssues),
			len(repoCtx.RepoState.OpenPRs))
	}

	return repoCtx, nil
}

func runCouncilStep(ctx context.Context, repoCtx *reposcanner.RepoContext) ([]domain.CouncilOutput, error) {
	if dryRun {
		fmt.Fprintln(os.Stderr, "dry-run: would run council perspectives")
		return []domain.CouncilOutput{
			{Councilor: "dry-run", Perspective: "placeholder"},
		}, nil
	}

	runner := &cruxexec.OSRunner{}
	registry := models.DefaultRegistry()
	spawner := council.NewSpawner(runner, registry)

	input := domain.CouncilInput{
		Vision:    repoCtx.Vision,
		RepoState: repoCtx.RepoState,
		Date:      time.Now().Format("2006-01-02"),
	}

	if verbose {
		fmt.Fprintln(os.Stderr, "Running council perspectives...")
	}

	results, err := spawner.RunCouncil(ctx, input)
	if err != nil {
		return nil, fmt.Errorf("council run: %w", err)
	}

	var outputs []domain.CouncilOutput
	for _, r := range results {
		if r.Output != nil {
			outputs = append(outputs, *r.Output)
		}
	}

	if verbose {
		fmt.Fprintf(os.Stderr, "Council complete: %d/%d perspectives succeeded\n",
			len(outputs), len(results))
	}

	return outputs, nil
}

func runSynthesisStep(ctx context.Context, councilOutputs []domain.CouncilOutput, visionText string) (*domain.SynthesisResult, error) {
	if dryRun {
		fmt.Fprintln(os.Stderr, "dry-run: would run synthesizer")
		return &domain.SynthesisResult{
			Synthesizer: "dry-run",
			Summary:     "Dry run synthesis",
			Items:       []domain.SynthesisItem{},
		}, nil
	}

	input := domain.SynthesisInput{
		CouncilOutputs: councilOutputs,
		Vision:         visionText,
	}

	runner := &cruxexec.OSRunner{}
	registry := models.DefaultRegistry()
	svc := synthesizer.NewService(runner, registry)

	if verbose {
		fmt.Fprintln(os.Stderr, "Running synthesizer...")
	}

	result, err := svc.Synthesize(ctx, input)
	if err != nil {
		return nil, fmt.Errorf("synthesis: %w", err)
	}

	if verbose {
		fmt.Fprintf(os.Stderr, "Synthesis complete: %d items\n", len(result.Items))
	}

	return result, nil
}

func emitResults(ctx context.Context, cmd *cobra.Command, result *domain.SynthesisResult, format output.Format) error {
	// Validate synthesis result
	if err := validateSynthesisResult(result); err != nil {
		return fmt.Errorf("validate synthesis result: %w", err)
	}

	// Append extra labels if provided (for github format)
	if len(groomLabels) > 0 {
		for i := range result.Items {
			result.Items[i].Labels = append(result.Items[i].Labels, groomLabels...)
		}
	}

	switch format {
	case output.GitHub:
		return emitGitHub(ctx, cmd, result)
	case output.Markdown:
		return emitMarkdown(cmd, result)
	case output.JSON:
		return emitJSON(cmd, result)
	case output.Stdout:
		return emitStdout(cmd, result)
	default:
		return fmt.Errorf("unhandled format: %s", format)
	}
}

func emitGitHub(ctx context.Context, cmd *cobra.Command, result *domain.SynthesisResult) error {
	if dryRun {
		output := ghissues.FormatDryRun(result.Items, ghissues.DefaultMilestones())
		fmt.Fprint(cmd.OutOrStdout(), output)
		return nil
	}

	runner := &cruxexec.OSRunner{}
	creator := &ghissues.Creator{
		Runner:     runner,
		Milestones: ghissues.DefaultMilestones(),
	}

	// Detect repo from git remote
	repo, err := detectRepo()
	if err != nil {
		return fmt.Errorf("detect repo: %w", err)
	}
	creator.Repo = repo

	if verbose {
		fmt.Fprintf(cmd.ErrOrStderr(), "Creating %d issues in %s\n", len(result.Items), repo)
	}

	created, err := creator.Create(ctx, result.Items)
	if err != nil {
		// Print any partial results
		for _, c := range created {
			fmt.Fprintf(cmd.OutOrStdout(), "Created: %s (%s)\n", c.Title, c.URL)
		}
		return fmt.Errorf("create issues: %w", err)
	}

	for _, c := range created {
		fmt.Fprintf(cmd.OutOrStdout(), "Created #%d: %s (%s)\n", c.Number, c.Title, c.URL)
	}

	fmt.Fprintf(cmd.OutOrStdout(), "\n%d issues created\n", len(created))
	return nil
}

func emitMarkdown(cmd *cobra.Command, result *domain.SynthesisResult) error {
	formatter := output.NewFormatter()
	content := formatter.FormatMarkdown(result)

	outputFile := filepath.Join(groomOutputDir, "backlog.md")
	if err := os.MkdirAll(groomOutputDir, 0o755); err != nil {
		return fmt.Errorf("create output dir: %w", err)
	}

	if err := os.WriteFile(outputFile, []byte(content), 0o644); err != nil {
		return fmt.Errorf("write markdown file: %w", err)
	}

	fmt.Fprintf(cmd.OutOrStdout(), "Wrote backlog to: %s\n", outputFile)
	return nil
}

func emitJSON(cmd *cobra.Command, result *domain.SynthesisResult) error {
	formatter := output.NewFormatter()
	content, err := formatter.FormatJSON(result)
	if err != nil {
		return err
	}

	outputFile := filepath.Join(groomOutputDir, "synthesis.json")
	if err := os.MkdirAll(groomOutputDir, 0o755); err != nil {
		return fmt.Errorf("create output dir: %w", err)
	}

	if err := os.WriteFile(outputFile, []byte(content), 0o644); err != nil {
		return fmt.Errorf("write JSON file: %w", err)
	}

	fmt.Fprintf(cmd.OutOrStdout(), "Wrote synthesis to: %s\n", outputFile)
	return nil
}

func emitStdout(cmd *cobra.Command, result *domain.SynthesisResult) error {
	formatter := output.NewFormatter()
	content := formatter.FormatStdout(result)
	fmt.Fprint(cmd.OutOrStdout(), content)
	return nil
}

func detectRepo() (string, error) {
	// First check if we're in a git repo with a GitHub remote
	runner := &cruxexec.OSRunner{}
	result, err := runner.Run(context.Background(), "git", []string{"remote", "get-url", "origin"}, cruxexec.RunOpts{})
	if err != nil {
		return "", fmt.Errorf("get git remote: %w", err)
	}

	if result.ExitCode != 0 {
		return "", fmt.Errorf("no git remote configured")
	}

	url := strings.TrimSpace(string(result.Stdout))
	// Parse github.com/owner/repo or git@github.com:owner/repo
	url = strings.TrimPrefix(url, "git@github.com:")
	url = strings.TrimPrefix(url, "https://github.com/")
	url = strings.TrimSuffix(url, ".git")

	parts := strings.Split(url, "/")
	if len(parts) != 2 {
		return "", fmt.Errorf("could not parse repo from remote: %s", url)
	}

	return parts[0] + "/" + parts[1], nil
}

// validateSynthesisResult validates the synthesis result structure.
func validateSynthesisResult(r *domain.SynthesisResult) error {
	for i, item := range r.Items {
		if item.Title == "" {
			return fmt.Errorf("item %d: title is required", i)
		}
		if !isValidPriority(item.Priority) {
			return fmt.Errorf("item %d: invalid priority %q", i, item.Priority)
		}
		if !isValidItemType(item.Type) {
			return fmt.Errorf("item %d: invalid type %q", i, item.Type)
		}
		if !isValidHorizon(item.Horizon) {
			return fmt.Errorf("item %d: invalid horizon %q", i, item.Horizon)
		}
	}
	return nil
}

// isValidPriority checks if priority is valid.
func isValidPriority(p domain.Priority) bool {
	switch p {
	case domain.P0, domain.P1, domain.P2, domain.P3:
		return true
	}
	return false
}

// isValidItemType checks if item type is valid.
func isValidItemType(t domain.ItemType) bool {
	switch t {
	case domain.Bug, domain.Feature, domain.Task, domain.Refactor, domain.Research:
		return true
	}
	return false
}

// isValidHorizon checks if horizon is valid.
func isValidHorizon(h domain.Horizon) bool {
	switch h {
	case domain.Now, domain.Next, domain.Later:
		return true
	}
	return false
}
