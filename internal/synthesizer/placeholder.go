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
type Placeholder struct {
	// ConfidenceThreshold is the minimum confidence for an item to be accepted.
	// Items below this threshold are dropped. Default: 0.5
	ConfidenceThreshold float64
}

var _ domain.Synthesizer = (*Placeholder)(nil)

// getThreshold returns the effective confidence threshold.
func (p *Placeholder) getThreshold() float64 {
	if p.ConfidenceThreshold <= 0 {
		return 0.5
	}
	return p.ConfidenceThreshold
}

func (p *Placeholder) Synthesize(_ context.Context, input domain.SynthesisInput) (*domain.SynthesisResult, error) {
	if len(input.CouncilOutputs) == 0 {
		return nil, fmt.Errorf("no council outputs provided")
	}

	threshold := p.getThreshold()

	// Track items and their support
	type itemData struct {
		item       *domain.SynthesisItem
		support    *domain.CouncilSupport
		confidence float64
	}

	seen := make(map[string]*itemData)
	var order []string

	totalCouncilors := len(input.CouncilOutputs)

	for _, co := range input.CouncilOutputs {
		for _, councilItem := range co.Items {
			key := councilItem.Title
			if data, ok := seen[key]; ok {
				// Merge: take higher priority
				if priorityRank(councilItem.Priority) < priorityRank(data.item.Priority) {
					data.item.Priority = councilItem.Priority
					data.item.Horizon = priorityToHorizon(councilItem.Priority)
				}
				// Track additional supporter
				data.support.ProposedBy = appendUnique(data.support.ProposedBy, co.Perspective)
			} else {
				si := &domain.SynthesisItem{
					Title:   councilItem.Title,
					Priority: councilItem.Priority,
					Type:    councilItem.Type,
					Horizon: priorityToHorizon(councilItem.Priority),
					Effort:  councilItem.Effort,
					Body:    fmt.Sprintf("## Rationale\n\n%s\n\n## Risk\n\n%s", councilItem.Rationale, councilItem.Risk),
					Labels:  []string{"source/groom"},
				}
				seen[key] = &itemData{
					item:    si,
					support: &domain.CouncilSupport{ProposedBy: []string{co.Perspective}},
				}
				order = append(order, key)
			}
		}
	}

	// Calculate confidence and separate items
	var items []domain.SynthesisItem
	var dropped []domain.DroppedItem

	for _, key := range order {
		data := seen[key]

		// Calculate consensus based on support ratio
		supportRatio := float64(len(data.support.ProposedBy)) / float64(totalCouncilors)
		data.support.Consensus = consensusFromRatio(supportRatio)

		// Calculate confidence based on multiple factors
		confidence := calculateConfidence(
			supportRatio,
			priorityRank(data.item.Priority),
			totalCouncilors,
		)
		data.item.Confidence = confidence
		data.item.CouncilSupport = *data.support

		if confidence >= threshold {
			items = append(items, *data.item)
		} else {
			droppedItem := domain.DroppedItem{
				Title:          data.item.Title,
				Reason:         generateDropReason(confidence, data.support),
				Confidence:     confidence,
				CouncilSupport: data.support.ProposedBy,
			}
			dropped = append(dropped, droppedItem)
		}
	}

	// Sort items by priority, then confidence
	sort.SliceStable(items, func(i, j int) bool {
		if priorityRank(items[i].Priority) != priorityRank(items[j].Priority) {
			return priorityRank(items[i].Priority) < priorityRank(items[j].Priority)
		}
		return items[i].Confidence > items[j].Confidence
	})

	return &domain.SynthesisResult{
		Synthesizer: "PLACEHOLDER",
		Model:       "none",
		Summary:     fmt.Sprintf("Merged %d items from %d council perspectives (%d dropped)", len(items), totalCouncilors, len(dropped)),
		Items:       items,
		Dropped:     dropped,
	}, nil
}

// calculateConfidence computes a confidence score (0.0-1.0) based on:
// - Support ratio (how many councilors proposed it)
// - Priority (higher priority = higher base confidence)
// - Number of councilors (more perspectives = more reliable)
func calculateConfidence(supportRatio float64, priorityRank, totalCouncilors int) float64 {
	// Base confidence from support ratio (0.0 - 0.6)
	baseConfidence := supportRatio * 0.6

	// Priority boost (0.0 - 0.25)
	// P0 gets max boost, P3 gets none
	priorityBoost := float64(3-priorityRank) * 0.08
	if priorityBoost < 0 {
		priorityBoost = 0
	}

	// Council size reliability factor (0.1 - 0.15)
	// More councilors = slightly more reliable
	reliabilityFactor := 0.1
	if totalCouncilors >= 4 {
		reliabilityFactor = 0.15
	} else if totalCouncilors >= 2 {
		reliabilityFactor = 0.12
	}

	confidence := baseConfidence + priorityBoost + reliabilityFactor
	if confidence > 1.0 {
		confidence = 1.0
	}
	return confidence
}

// generateDropReason creates a human-readable explanation for why an item was dropped.
func generateDropReason(confidence float64, support *domain.CouncilSupport) string {
	switch {
	case confidence < 0.3:
		return "low council support and alignment"
	case confidence < 0.5:
		if len(support.ProposedBy) == 1 {
			return "single councilor support, insufficient consensus"
		}
		return "insufficient consensus across council"
	default:
		return "below confidence threshold"
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

func consensusFromRatio(ratio float64) domain.Consensus {
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
