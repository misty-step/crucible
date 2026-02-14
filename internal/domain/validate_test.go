package domain

import (
	"strings"
	"testing"
)

func TestPriorityValid(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name string
		p    Priority
		want bool
	}{
		{"p0", P0, true},
		{"p1", P1, true},
		{"p2", P2, true},
		{"p3", P3, true},
		{"empty", Priority(""), false},
		{"unknown", Priority("p4"), false},
		{"wrong_case", Priority("P0"), false},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := tc.p.Valid(); got != tc.want {
				t.Fatalf("Valid() = %v, want %v", got, tc.want)
			}
		})
	}
}

func TestItemTypeValid(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name string
		typ  ItemType
		want bool
	}{
		{"bug", Bug, true},
		{"feature", Feature, true},
		{"task", Task, true},
		{"refactor", Refactor, true},
		{"research", Research, true},
		{"empty", ItemType(""), false},
		{"unknown", ItemType("chore"), false},
		{"wrong_case", ItemType("Bug"), false},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := tc.typ.Valid(); got != tc.want {
				t.Fatalf("Valid() = %v, want %v", got, tc.want)
			}
		})
	}
}

func TestEffortValid(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name string
		e    Effort
		want bool
	}{
		{"s", Small, true},
		{"m", Medium, true},
		{"l", Large, true},
		{"xl", ExtraLarge, true},
		{"empty", Effort(""), false},
		{"unknown", Effort("xxl"), false},
		{"wrong_case", Effort("XL"), false},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := tc.e.Valid(); got != tc.want {
				t.Fatalf("Valid() = %v, want %v", got, tc.want)
			}
		})
	}
}

func TestHorizonValid(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name string
		h    Horizon
		want bool
	}{
		{"now", Now, true},
		{"next", Next, true},
		{"later", Later, true},
		{"empty", Horizon(""), false},
		{"unknown", Horizon("soon"), false},
		{"wrong_case", Horizon("Now"), false},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := tc.h.Valid(); got != tc.want {
				t.Fatalf("Valid() = %v, want %v", got, tc.want)
			}
		})
	}
}

func TestCouncilItemValidate(t *testing.T) {
	t.Parallel()

	valid := CouncilItem{
		Title:    "Fix flaky tests",
		Priority: P1,
		Type:     Bug,
		Effort:   Small,
	}

	tests := []struct {
		name        string
		item        CouncilItem
		wantErr     bool
		errContains string
	}{
		{"valid", valid, false, ""},
		{"empty_title", func() CouncilItem { c := valid; c.Title = ""; return c }(), true, "title is required"},
		{"title_too_long", func() CouncilItem { c := valid; c.Title = strings.Repeat("a", 201); return c }(), true, "title exceeds 200 characters"},
		{"invalid_priority", func() CouncilItem { c := valid; c.Priority = Priority("p9"); return c }(), true, "invalid priority"},
		{"invalid_type", func() CouncilItem { c := valid; c.Type = ItemType("chore"); return c }(), true, "invalid type"},
		{"invalid_effort", func() CouncilItem { c := valid; c.Effort = Effort("xxl"); return c }(), true, "invalid effort"},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()

			err := tc.item.Validate()
			if (err != nil) != tc.wantErr {
				t.Fatalf("Validate() err = %v, wantErr %v", err, tc.wantErr)
			}
			if tc.wantErr && tc.errContains != "" && !strings.Contains(err.Error(), tc.errContains) {
				t.Fatalf("Validate() err = %q, want contains %q", err.Error(), tc.errContains)
			}
		})
	}
}

func TestCouncilOutputValidate(t *testing.T) {
	t.Parallel()

	valid := CouncilOutput{
		Councilor:   "product",
		Perspective: "product",
		Confidence:  0.8,
		Items: []CouncilItem{
			{
				Title:    "Add help text for flags",
				Priority: P2,
				Type:     Task,
				Effort:   Small,
			},
		},
	}

	tests := []struct {
		name        string
		out         CouncilOutput
		wantErr     bool
		errContains string
	}{
		{"valid", valid, false, ""},
		{"missing_councilor", func() CouncilOutput { c := valid; c.Councilor = ""; return c }(), true, "councilor is required"},
		{"missing_perspective", func() CouncilOutput { c := valid; c.Perspective = ""; return c }(), true, "perspective is required"},
		{"confidence_too_high", func() CouncilOutput { c := valid; c.Confidence = 1.1; return c }(), true, "confidence must be 0.0-1.0"},
		{"confidence_negative", func() CouncilOutput { c := valid; c.Confidence = -0.1; return c }(), true, "confidence must be 0.0-1.0"},
		{"invalid_item", func() CouncilOutput {
			c := valid
			c.Items = []CouncilItem{{Title: "", Priority: P1, Type: Feature, Effort: Medium}}
			return c
		}(), true, "items[0]:"},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()

			err := tc.out.Validate()
			if (err != nil) != tc.wantErr {
				t.Fatalf("Validate() err = %v, wantErr %v", err, tc.wantErr)
			}
			if tc.wantErr && tc.errContains != "" && !strings.Contains(err.Error(), tc.errContains) {
				t.Fatalf("Validate() err = %q, want contains %q", err.Error(), tc.errContains)
			}
		})
	}
}

func TestSynthesisItemValidate(t *testing.T) {
	t.Parallel()

	valid := SynthesisItem{
		Title:    "Ship v1 council command",
		Priority: P1,
		Type:     Feature,
		Horizon:  Now,
		Effort:   Medium,
		Body:     "Implement core council invocation and output collection.",
	}

	tests := []struct {
		name        string
		item        SynthesisItem
		wantErr     bool
		errContains string
	}{
		{"valid", valid, false, ""},
		{"empty_title", func() SynthesisItem { s := valid; s.Title = ""; return s }(), true, "title is required"},
		{"invalid_priority", func() SynthesisItem { s := valid; s.Priority = Priority("p9"); return s }(), true, "invalid priority"},
		{"invalid_type", func() SynthesisItem { s := valid; s.Type = ItemType("chore"); return s }(), true, "invalid type"},
		{"invalid_horizon", func() SynthesisItem { s := valid; s.Horizon = Horizon("soon"); return s }(), true, "invalid horizon"},
		{"invalid_effort", func() SynthesisItem { s := valid; s.Effort = Effort("xxl"); return s }(), true, "invalid effort"},
		{"empty_body", func() SynthesisItem { s := valid; s.Body = ""; return s }(), true, "body is required"},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()

			err := tc.item.Validate()
			if (err != nil) != tc.wantErr {
				t.Fatalf("Validate() err = %v, wantErr %v", err, tc.wantErr)
			}
			if tc.wantErr && tc.errContains != "" && !strings.Contains(err.Error(), tc.errContains) {
				t.Fatalf("Validate() err = %q, want contains %q", err.Error(), tc.errContains)
			}
		})
	}
}

func TestSynthesisResultValidate(t *testing.T) {
	t.Parallel()

	t.Run("valid", func(t *testing.T) {
		t.Parallel()

		got := SynthesisResult{
			Synthesizer: "oracle",
			Model:       "gpt-x",
			Summary:     "A small focused backlog",
			Items: []SynthesisItem{
				{
					Title:    "Add README quickstart",
					Priority: P2,
					Type:     Task,
					Horizon:  Next,
					Effort:   Small,
					Body:     "Document install + auth prerequisites.",
				},
			},
		}

		if err := got.Validate(); err != nil {
			t.Fatalf("Validate() err = %v, want nil", err)
		}
	})

	t.Run("invalid_item", func(t *testing.T) {
		t.Parallel()

		got := SynthesisResult{
			Items: []SynthesisItem{
				{
					Title:    "",
					Priority: P1,
					Type:     Feature,
					Horizon:  Now,
					Effort:   Medium,
					Body:     "Body",
				},
			},
		}

		err := got.Validate()
		if err == nil {
			t.Fatalf("Validate() err = nil, want error")
		}
		if !strings.Contains(err.Error(), "items[0]:") {
			t.Fatalf("Validate() err = %q, want contains %q", err.Error(), "items[0]:")
		}
	})
}
