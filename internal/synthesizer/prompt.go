package synthesizer

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/misty-step/crucible/internal/domain"
)

// RenderSynthesisPrompt builds the user message for the ORACLE agent.
func RenderSynthesisPrompt(input domain.SynthesisInput) string {
	var b strings.Builder

	b.WriteString("# Synthesis Input\n\n")

	if input.Vision != "" {
		b.WriteString("## Vision\n\n")
		b.WriteString(input.Vision)
		b.WriteString("\n\n")
	}

	if input.RepoContext != "" {
		b.WriteString("## Repository Context\n\n")
		b.WriteString(input.RepoContext)
		b.WriteString("\n\n")
	}

	b.WriteString("## Council Outputs\n\n")
	for i, co := range input.CouncilOutputs {
		fmt.Fprintf(&b, "### %s (%s)\n\n", co.Councilor, co.Perspective)
		data, err := json.MarshalIndent(co, "", "  ")
		if err != nil {
			fmt.Fprintf(&b, "(failed to serialize council output %d: %v)\n\n", i, err)
			continue
		}
		fmt.Fprintf(&b, "```json\n%s\n```\n\n", string(data))
	}

	b.WriteString("## Task\n\n")
	b.WriteString("Reconcile these council outputs. Merge overlapping items, resolve conflicts, drop weak proposals. Return your synthesis as a JSON block matching the schema in your system prompt.\n")

	return b.String()
}
