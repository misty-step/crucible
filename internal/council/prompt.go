package council

import (
	"fmt"
	"strings"

	"github.com/misty-step/crucible/internal/domain"
)

// RenderPrompt builds the user message sent to each council agent.
// The agent definition file provides the system prompt; this is the input.
func RenderPrompt(input domain.CouncilInput) string {
	var b strings.Builder

	b.WriteString("# Council Deliberation Input\n\n")
	fmt.Fprintf(&b, "Date: %s\n\n", input.Date)

	if input.Vision != "" {
		b.WriteString("## Vision\n\n")
		b.WriteString(input.Vision)
		b.WriteString("\n\n")
	}

	if input.RepoState.FileTree != "" || len(input.RepoState.RecentCommits) > 0 ||
		len(input.RepoState.OpenIssues) > 0 || len(input.RepoState.OpenPRs) > 0 {
		b.WriteString("## Repository State\n\n")

		if len(input.RepoState.RecentCommits) > 0 {
			b.WriteString("### Recent Commits\n")
			for _, c := range input.RepoState.RecentCommits {
				fmt.Fprintf(&b, "- %s\n", c)
			}
			b.WriteString("\n")
		}

		if len(input.RepoState.OpenIssues) > 0 {
			b.WriteString("### Open Issues\n")
			for _, i := range input.RepoState.OpenIssues {
				fmt.Fprintf(&b, "- %s\n", i)
			}
			b.WriteString("\n")
		}

		if len(input.RepoState.OpenPRs) > 0 {
			b.WriteString("### Open PRs\n")
			for _, pr := range input.RepoState.OpenPRs {
				fmt.Fprintf(&b, "- %s\n", pr)
			}
			b.WriteString("\n")
		}

		if input.RepoState.FileTree != "" {
			b.WriteString("### File Tree\n```\n")
			b.WriteString(input.RepoState.FileTree)
			b.WriteString("\n```\n\n")
		}
	}

	if input.HumanInput != "" {
		b.WriteString("## Human Input\n\n")
		b.WriteString(input.HumanInput)
		b.WriteString("\n\n")
	}

	b.WriteString("## Task\n\n")
	b.WriteString("Evaluate this project from your perspective. Return your assessment as a JSON block matching the schema in your system prompt.\n")

	return b.String()
}
