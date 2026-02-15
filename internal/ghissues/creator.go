package ghissues

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
)

const issueDelay = 1 * time.Second

// MilestoneMap maps horizon values to GitHub milestone names.
type MilestoneMap map[domain.Horizon]string

// DefaultMilestones returns the standard horizon-to-milestone mapping.
func DefaultMilestones() MilestoneMap {
	return MilestoneMap{
		domain.Now:   "v0.2.0 — Core Implementation",
		domain.Next:  "v0.2.0 — Core Implementation",
		domain.Later: "Backlog",
	}
}

// Creator creates GitHub issues from synthesis output via the gh CLI.
type Creator struct {
	Runner     cruxexec.CommandRunner
	Milestones MilestoneMap
	Repo       string // owner/name, empty = current repo
	DryRun     bool
}

// Create implements the Emitter pattern: converts synthesis items into GitHub issues.
func (c *Creator) Create(ctx context.Context, items []domain.SynthesisItem) ([]domain.CreatedIssue, error) {
	var created []domain.CreatedIssue

	for i, item := range items {
		if i > 0 && !c.DryRun {
			select {
			case <-ctx.Done():
				return created, ctx.Err()
			case <-time.After(issueDelay):
			}
		}

		issue, err := c.createOne(ctx, item)
		if err != nil {
			return created, fmt.Errorf("item %d (%s): %w", i, item.Title, err)
		}
		created = append(created, issue)
	}

	return created, nil
}

func (c *Creator) createOne(ctx context.Context, item domain.SynthesisItem) (domain.CreatedIssue, error) {
	labels := c.buildLabels(item)
	body := c.buildBody(item)

	args := []string{
		"issue", "create",
		"--title", item.Title,
		"--body", body,
		"--label", strings.Join(labels, ","),
	}

	if ms, ok := c.Milestones[item.Horizon]; ok {
		args = append(args, "--milestone", ms)
	}

	if c.Repo != "" {
		args = append(args, "--repo", c.Repo)
	}

	result, err := c.Runner.Run(ctx, "gh", args, cruxexec.RunOpts{
		Timeout: 30 * time.Second,
	})
	if err != nil {
		return domain.CreatedIssue{}, fmt.Errorf("gh issue create: %w", err)
	}

	if result.ExitCode != 0 {
		return domain.CreatedIssue{}, fmt.Errorf("gh issue create exited %d: %s",
			result.ExitCode, truncate(string(result.Stderr), 200))
	}

	url := strings.TrimSpace(string(result.Stdout))
	number := extractIssueNumber(url)

	return domain.CreatedIssue{
		Number: number,
		URL:    url,
		Title:  item.Title,
	}, nil
}

func (c *Creator) buildLabels(item domain.SynthesisItem) []string {
	labels := []string{
		string(item.Priority),
		string(item.Type),
		string(item.Horizon),
		"source/groom",
	}

	if item.Effort != "" {
		labels = append(labels, "effort/"+string(item.Effort))
	}

	labels = append(labels, item.Labels...)
	return labels
}

func (c *Creator) buildBody(item domain.SynthesisItem) string {
	var b strings.Builder

	b.WriteString(item.Body)
	b.WriteString("\n\n---\n")

	if len(item.CouncilSupport.ProposedBy) > 0 {
		fmt.Fprintf(&b, "\n**Council support:** %s (consensus: %s)\n",
			strings.Join(item.CouncilSupport.ProposedBy, ", "),
			item.CouncilSupport.Consensus)
	}

	if item.VisionAlignment != "" {
		fmt.Fprintf(&b, "\n**Vision alignment:** %s\n", item.VisionAlignment)
	}

	b.WriteString("\n*Created by crucible*\n")
	return b.String()
}

// FormatDryRun renders items as markdown for review instead of creating issues.
func FormatDryRun(items []domain.SynthesisItem, milestones MilestoneMap) string {
	var b strings.Builder

	fmt.Fprintf(&b, "# Dry Run: %d issues\n\n", len(items))

	for i, item := range items {
		labels := []string{
			string(item.Priority),
			string(item.Type),
			string(item.Horizon),
			"source/groom",
		}
		if item.Effort != "" {
			labels = append(labels, "effort/"+string(item.Effort))
		}
		labels = append(labels, item.Labels...)

		ms := milestones[item.Horizon]

		fmt.Fprintf(&b, "## %d. %s\n\n", i+1, item.Title)
		fmt.Fprintf(&b, "**Labels:** %s\n", strings.Join(labels, ", "))
		fmt.Fprintf(&b, "**Milestone:** %s\n\n", ms)
		b.WriteString(item.Body)
		b.WriteString("\n\n---\n\n")
	}

	return b.String()
}

// ghIssueOutput is the JSON structure from `gh issue create --json`
type ghIssueOutput struct {
	Number int    `json:"number"`
	URL    string `json:"url"`
}

func extractIssueNumber(urlOrJSON string) int {
	// gh outputs the issue URL on stdout
	s := strings.TrimSpace(urlOrJSON)

	// Try JSON first (future-proofing if --json flag is used)
	var out ghIssueOutput
	if json.Unmarshal([]byte(s), &out) == nil && out.Number > 0 {
		return out.Number
	}

	// Parse from URL: https://github.com/owner/repo/issues/42
	parts := strings.Split(s, "/")
	if len(parts) >= 2 {
		var n int
		_, _ = fmt.Sscanf(parts[len(parts)-1], "%d", &n)
		return n
	}

	return 0
}

func truncate(s string, maxLen int) string {
	s = strings.TrimSpace(s)
	if len(s) > maxLen {
		return s[:maxLen] + "..."
	}
	return s
}
