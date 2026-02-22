// Package report provides synthesis reporting and output formatting.
package report

import (
	"fmt"
	"io"
	"sort"
	"strings"

	"github.com/misty-step/crucible/internal/domain"
)

// SummaryReporter generates human-readable synthesis summaries.
type SummaryReporter struct {
	w io.Writer
}

// NewSummaryReporter creates a new reporter that writes to w.
func NewSummaryReporter(w io.Writer) *SummaryReporter {
	return &SummaryReporter{w: w}
}

// PrintSynthesisSummary prints a formatted summary of synthesis results.
func (r *SummaryReporter) PrintSynthesisSummary(result *domain.SynthesisResult) {
	fmt.Fprintf(r.w, "\n%s\n", strings.Repeat("=", 60))
	fmt.Fprintf(r.w, "SYNTHESIS SUMMARY: %s\n", result.Summary)
	fmt.Fprintf(r.w, "%s\n\n", strings.Repeat("=", 60))

	r.printDroppedItems(result.Dropped)
	r.printBorderlineItems(result.Items)
	r.printAcceptedItems(result.Items)
}

func (r *SummaryReporter) printDroppedItems(dropped []domain.DroppedItem) {
	if len(dropped) == 0 {
		return
	}

	fmt.Fprintf(r.w, "DROPPED (%d items):\n", len(dropped))

	// Sort by confidence ascending (lowest first)
	sorted := make([]domain.DroppedItem, len(dropped))
	copy(sorted, dropped)
	sort.Slice(sorted, func(i, j int) bool {
		return sorted[i].Confidence < sorted[j].Confidence
	})

	for _, item := range sorted {
		symbol := r.symbolForConfidence(item.Confidence)
		fmt.Fprintf(r.w, "  %s \"%s\" — %s (confidence: %.2f)\n",
			symbol, item.Title, item.Reason, item.Confidence)

		if len(item.CouncilSupport) > 0 {
			fmt.Fprintf(r.w, "      support: %s\n", strings.Join(item.CouncilSupport, ", "))
		}
	}
	fmt.Fprintln(r.w)
}

func (r *SummaryReporter) printBorderlineItems(items []domain.SynthesisItem) {
	var borderline []domain.SynthesisItem
	for _, item := range items {
		if item.IsBorderline() {
			borderline = append(borderline, item)
		}
	}

	if len(borderline) == 0 {
		return
	}

	fmt.Fprintf(r.w, "BORDERLINE (%d items):\n", len(borderline))

	// Sort by confidence descending (highest borderline first = closest to acceptance)
	sort.Slice(borderline, func(i, j int) bool {
		return borderline[i].Confidence > borderline[j].Confidence
	})

	for _, item := range borderline {
		proposedBy := strings.Join(item.CouncilSupport.ProposedBy, ", ")
		fmt.Fprintf(r.w, "  ? \"%s\" — proposed by %s (confidence: %.2f)\n",
			item.Title, proposedBy, item.Confidence)
	}
	fmt.Fprintln(r.w)
}

func (r *SummaryReporter) printAcceptedItems(items []domain.SynthesisItem) {
	accepted := make([]domain.SynthesisItem, 0, len(items))
	for _, item := range items {
		if !item.IsBorderline() {
			accepted = append(accepted, item)
		}
	}

	if len(accepted) == 0 {
		return
	}

	fmt.Fprintf(r.w, "ACCEPTED (%d items):\n", len(accepted))

	// Sort by priority then confidence
	sort.Slice(accepted, func(i, j int) bool {
		if accepted[i].Priority != accepted[j].Priority {
			return priorityRank(accepted[i].Priority) < priorityRank(accepted[j].Priority)
		}
		return accepted[i].Confidence > accepted[j].Confidence
	})

	for _, item := range accepted {
		fmt.Fprintf(r.w, "  ✓ \"%s\" — %s/%s (confidence: %.2f)\n",
			item.Title, item.Priority, item.Horizon, item.Confidence)
	}
	fmt.Fprintln(r.w)
}

func (r *SummaryReporter) symbolForConfidence(confidence float64) string {
	switch {
	case confidence < 0.3:
		return "✗"
	case confidence < 0.5:
		return "⚠"
	default:
		return "~"
	}
}

func priorityRank(p domain.Priority) int {
	switch p {
	case domain.P0:
		return 0
	case domain.P1:
		return 1
	case domain.P2:
		return 2
	case domain.P3:
		return 3
	default:
		return 99
	}
}
