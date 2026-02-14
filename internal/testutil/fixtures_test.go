package testutil

import (
	"testing"
)

func TestLoadFixture(t *testing.T) {
	t.Parallel()

	data := LoadFixture(t, "council_output_strategist.json")
	if len(data) == 0 {
		t.Fatal("fixture is empty")
	}
}

func TestMustParseCouncilOutputValid(t *testing.T) {
	t.Parallel()

	out := MustParseCouncilOutput(t, "council_output_strategist.json")
	if out.Councilor != "STRATEGIST" {
		t.Fatalf("got councilor %q, want %q", out.Councilor, "STRATEGIST")
	}
	if out.Perspective != "product" {
		t.Fatalf("got perspective %q, want %q", out.Perspective, "product")
	}
	if len(out.Items) != 2 {
		t.Fatalf("got %d items, want 2", len(out.Items))
	}
}

func TestMustParseCouncilOutputArchitect(t *testing.T) {
	t.Parallel()

	out := MustParseCouncilOutput(t, "council_output_architect.json")
	if out.Councilor != "ARCHITECT" {
		t.Fatalf("got councilor %q, want %q", out.Councilor, "ARCHITECT")
	}
	if len(out.Items) != 3 {
		t.Fatalf("got %d items, want 3", len(out.Items))
	}
}

func TestMustParseSynthesisResult(t *testing.T) {
	t.Parallel()

	result := MustParseSynthesisResult(t, "synthesis_result.json")
	if result.Synthesizer != "ORACLE" {
		t.Fatalf("got synthesizer %q, want %q", result.Synthesizer, "ORACLE")
	}
	if len(result.Items) != 2 {
		t.Fatalf("got %d items, want 2", len(result.Items))
	}
	if len(result.Dropped) != 1 {
		t.Fatalf("got %d dropped, want 1", len(result.Dropped))
	}
}

func TestMustParseCouncilOutputInvalid(t *testing.T) {
	t.Parallel()

	// Should parse (it's valid JSON) but fail validation
	out := MustParseCouncilOutput(t, "council_output_invalid.json")
	if err := out.Validate(); err == nil {
		t.Fatal("expected validation error for invalid fixture")
	}
}
