// Package output provides formatters for different output modes.
package output

import (
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/misty-step/crucible/internal/domain"
	"github.com/misty-step/crucible/internal/ghissues"
)

// Format defines the output format type.
type Format string

const (
	// GitHub creates GitHub issues (default).
	GitHub Format = "github"
	// Markdown writes a prioritized backlog document.
	Markdown Format = "markdown"
	// JSON outputs raw synthesis result.
	JSON Format = "json"
	// Stdout prints human-readable summary.
	Stdout Format = "stdout"
)

// Valid checks if the format is valid.
func (f Format) Valid() bool {
	switch f {
	case GitHub, Markdown, JSON, Stdout:
		return true
	}
	return false
}

// Formatter formats synthesis results for different output modes.
type Formatter struct {
	Milestones ghissues.MilestoneMap
}

// NewFormatter creates a formatter with default milestones.
func NewFormatter() *Formatter {
	return &Formatter{
		Milestones: ghissues.DefaultMilestones(),
	}
}

// FormatResult formats the synthesis result according to the given format.
func (f *Formatter) FormatResult(result *domain.SynthesisResult, format Format) (string, error) {
	switch format {
	case GitHub:
		return "", fmt.Errorf("github format requires issue creation, use emitter")
	case Markdown:
		return f.FormatMarkdown(result), nil
	case JSON:
		return f.FormatJSON(result)
	case Stdout:
		return f.FormatStdout(result), nil
	default:
		return "", fmt.Errorf("unknown format: %s", format)
	}
}

// FormatMarkdown renders a prioritized backlog as markdown.
func (f *Formatter) FormatMarkdown(result *domain.SynthesisResult) string {
	var b strings.Builder

	fmt.Fprintf(&b, "# Groomed Backlog: %s\n\n", result.Summary)
	fmt.Fprintf(&b, "Generated: %s\n\n", time.Now().Format("2006-01-02 15:04"))

	if len(result.Items) == 0 {
		b.WriteString("*No backlog items generated.*\n")
		return b.String()
	}

	// Group by horizon
	byHorizon := groupByHorizon(result.Items)
	horizons := []domain.Horizon{domain.Now, domain.Next, domain.Later}
	horizonTitles := map[domain.Horizon]string{
		domain.Now:   "Now",
		domain.Next:  "Next",
		domain.Later: "Later",
	}

	for _, h := range horizons {
		items := byHorizon[h]
		if len(items) == 0 {
			continue
		}

		fmt.Fprintf(&b, "## %s\n\n", horizonTitles[h])

		for i, item := range items {
			f.formatMarkdownItem(&b, i+1, item)
		}
	}

	if len(result.Conflicts) > 0 {
		b.WriteString("\n---\n\n## Conflicts Resolved\n\n")
		for _, c := range result.Conflicts {
			fmt.Fprintf(&b, "### %s\n\n", c.Item)
			fmt.Fprintf(&b, "**Disagreement:** %s\n", c.Disagreement)
			fmt.Fprintf(&b, "**Resolution:** %s\n\n", c.Resolution)
		}
	}

	if len(result.Dropped) > 0 {
		b.WriteString("\n---\n\n## Dropped Items\n\n")
		for _, d := range result.Dropped {
			fmt.Fprintf(&b, "- **%s**: %s\n", d.Title, d.Reason)
		}
	}

	return b.String()
}

// formatMarkdownItem formats a single item for markdown output.
func (f *Formatter) formatMarkdownItem(b *strings.Builder, num int, item domain.SynthesisItem) {
	fmt.Fprintf(b, "### %d. %s\n\n", num, item.Title)

	// Metadata line
	var meta []string
	meta = append(meta, fmt.Sprintf("**Priority:** %s", item.Priority))
	meta = append(meta, fmt.Sprintf("**Type:** %s", item.Type))
	meta = append(meta, fmt.Sprintf("**Horizon:** %s", item.Horizon))
	if item.Effort != "" {
		meta = append(meta, fmt.Sprintf("**Effort:** %s", item.Effort))
	}

	labels := f.buildLabels(item)
	if len(labels) > 0 {
		meta = append(meta, fmt.Sprintf("**Labels:** %s", strings.Join(labels, ", ")))
	}

	if ms, ok := f.Milestones[item.Horizon]; ok {
		meta = append(meta, fmt.Sprintf("**Milestone:** %s", ms))
	}

	b.WriteString(strings.Join(meta, " | ") + "\n\n")

	// Body
	if item.Body != "" {
		b.WriteString(item.Body)
		b.WriteString("\n\n")
	}

	// Council support
	if len(item.CouncilSupport.ProposedBy) > 0 {
		fmt.Fprintf(b, "**Council support:** %s", strings.Join(item.CouncilSupport.ProposedBy, ", "))
		if item.CouncilSupport.Consensus != "" {
			fmt.Fprintf(b, " (consensus: %s)", item.CouncilSupport.Consensus)
		}
		b.WriteString("\n\n")
	}

	if item.VisionAlignment != "" {
		fmt.Fprintf(b, "**Vision alignment:** %s\n\n", item.VisionAlignment)
	}
}

// FormatJSON outputs raw synthesis result as JSON.
func (f *Formatter) FormatJSON(result *domain.SynthesisResult) (string, error) {
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return "", fmt.Errorf("marshal result: %w", err)
	}
	return string(data), nil
}

// FormatStdout prints a human-readable summary.
func (f *Formatter) FormatStdout(result *domain.SynthesisResult) string {
	var b strings.Builder

	fmt.Fprintf(&b, "Groomed Backlog: %s\n", result.Summary)
	fmt.Fprintf(&b, "Synthesizer: %s\n", result.Synthesizer)
	if result.Model != "" {
		fmt.Fprintf(&b, "Model: %s\n", result.Model)
	}
	fmt.Fprintf(&b, "\n%d items generated\n\n", len(result.Items))

	if len(result.Items) == 0 {
		return b.String()
	}

	// Group by priority
	byPriority := groupByPriority(result.Items)
	priorities := []domain.Priority{domain.P0, domain.P1, domain.P2, domain.P3}
	priorityNames := map[domain.Priority]string{
		domain.P0: "P0 - Critical",
		domain.P1: "P1 - High",
		domain.P2: "P2 - Medium",
		domain.P3: "P3 - Low",
	}

	for _, p := range priorities {
		items := byPriority[p]
		if len(items) == 0 {
			continue
		}

		fmt.Fprintf(&b, "=== %s (%d items) ===\n", priorityNames[p], len(items))
		for _, item := range items {
			horizon := ""
			if item.Horizon != "" {
				horizon = fmt.Sprintf(" [%s]", item.Horizon)
			}
			effort := ""
			if item.Effort != "" {
				effort = fmt.Sprintf(" (%s)", item.Effort)
			}
			fmt.Fprintf(&b, "  • %s%s%s\n", item.Title, horizon, effort)
		}
		b.WriteString("\n")
	}

	if len(result.Conflicts) > 0 {
		fmt.Fprintf(&b, "Conflicts resolved: %d\n", len(result.Conflicts))
	}

	if len(result.Dropped) > 0 {
		fmt.Fprintf(&b, "Items dropped: %d\n", len(result.Dropped))
	}

	return b.String()
}

// buildLabels constructs the label list for display.
func (f *Formatter) buildLabels(item domain.SynthesisItem) []string {
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

func groupByHorizon(items []domain.SynthesisItem) map[domain.Horizon][]domain.SynthesisItem {
	result := make(map[domain.Horizon][]domain.SynthesisItem)
	for _, item := range items {
		result[item.Horizon] = append(result[item.Horizon], item)
	}
	return result
}

func groupByPriority(items []domain.SynthesisItem) map[domain.Priority][]domain.SynthesisItem {
	result := make(map[domain.Priority][]domain.SynthesisItem)
	for _, item := range items {
		result[item.Priority] = append(result[item.Priority], item)
	}
	return result
}
