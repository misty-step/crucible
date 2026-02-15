package synthesizer

import (
	"context"
	"fmt"
	"sort"

	"github.com/misty-step/crucible/internal/domain"
)

// Placeholder implements domain.Synthesizer by merging and deduplicating
// council outputs without calling an LLM. It's a stand-in until the real
// synthesizer (which uses claude-opus-4-6) is merged.
type Placeholder struct{}

var _ domain.Synthesizer = (*Placeholder)(nil)

func (p *Placeholder) Synthesize(_ context.Context, input domain.SynthesisInput) (*domain.SynthesisResult, error) {
	if len(input.CouncilOutputs) == 0 {
		return nil, fmt.Errorf("no council outputs provided")
	}

	seen := make(map[string]*domain.SynthesisItem)
	support := make(map[string]*domain.CouncilSupport)
	var order []string

	for _, co := range input.CouncilOutputs {
		for _, item := range co.Items {
			key := item.Title
			if existing, ok := seen[key]; ok {
				// Merge: take higher priority
				if priorityRank(item.Priority) < priorityRank(existing.Priority) {
					existing.Priority = item.Priority
				}
				// Track additional supporter
				sup := support[key]
				sup.ProposedBy = appendUnique(sup.ProposedBy, co.Perspective)
			} else {
				si := domain.SynthesisItem{
					Title:    item.Title,
					Priority: item.Priority,
					Type:     item.Type,
					Horizon:  priorityToHorizon(item.Priority),
					Effort:   item.Effort,
					Body:     fmt.Sprintf("## Rationale\n\n%s\n\n## Risk\n\n%s", item.Rationale, item.Risk),
					Labels:   []string{"source/groom"},
				}
				seen[key] = &si
				support[key] = &domain.CouncilSupport{
					ProposedBy: []string{co.Perspective},
				}
				order = append(order, key)
			}
		}
	}

	// Build final items in original encounter order, sorted by priority
	items := make([]domain.SynthesisItem, 0, len(order))
	for _, key := range order {
		item := seen[key]
		sup := support[key]
		sup.Consensus = consensusFromCount(len(sup.ProposedBy), len(input.CouncilOutputs))
		item.CouncilSupport = *sup
		items = append(items, *item)
	}

	sort.SliceStable(items, func(i, j int) bool {
		return priorityRank(items[i].Priority) < priorityRank(items[j].Priority)
	})

	return &domain.SynthesisResult{
		Synthesizer: "PLACEHOLDER",
		Model:       "none",
		Summary:     fmt.Sprintf("Merged %d items from %d council perspectives", len(items), len(input.CouncilOutputs)),
		Items:       items,
	}, nil
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

func priorityToHorizon(p domain.Priority) domain.Horizon {
	switch p {
	case domain.P0:
		return domain.Now
	case domain.P1:
		return domain.Next
	default:
		return domain.Later
	}
}

func consensusFromCount(supporters, total int) domain.Consensus {
	if total == 0 {
		return domain.Split
	}
	ratio := float64(supporters) / float64(total)
	if ratio >= 0.75 {
		return domain.Strong
	}
	if ratio >= 0.5 {
		return domain.Moderate
	}
	return domain.Split
}

func appendUnique(slice []string, val string) []string {
	for _, s := range slice {
		if s == val {
			return slice
		}
	}
	return append(slice, val)
}
