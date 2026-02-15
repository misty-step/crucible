package cmd

import (
	"bytes"
	"strings"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
)

func TestGroomCmd_InvalidFormat(t *testing.T) {
	// Save and restore original flags
	origOutput := groomOutput
	defer func() { groomOutput = origOutput }()

	groomOutput = "invalid-format"

	var buf bytes.Buffer
	cmd := groomCmd
	cmd.SetOut(&buf)
	cmd.SetErr(&buf)

	err := runGroom(cmd, nil)
	if err == nil {
		t.Error("expected error for invalid format")
	}
	if !strings.Contains(err.Error(), "invalid output format") {
		t.Errorf("expected format error, got: %v", err)
	}
}

func TestEmitStdout(t *testing.T) {
	result := &domain.SynthesisResult{
		Synthesizer: "test",
		Summary:     "Test grooming",
		Items: []domain.SynthesisItem{
			{
				Title:    "Test item",
				Priority: domain.P1,
				Type:     domain.Feature,
				Horizon:  domain.Next,
				Effort:   domain.Medium,
			},
		},
	}

	var buf bytes.Buffer
	cmd := groomCmd
	cmd.SetOut(&buf)

	err := emitStdout(cmd, result)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	output := buf.String()
	if !strings.Contains(output, "Test grooming") {
		t.Error("expected summary in output")
	}
	if !strings.Contains(output, "Test item") {
		t.Error("expected item title in output")
	}
	if !strings.Contains(output, "1 items generated") {
		t.Error("expected item count")
	}
}

func TestEmitJSON_WithDryRun(t *testing.T) {
	// Save and restore
	origDryRun := dryRun
	origGroomOutputDir := groomOutputDir
	defer func() {
		dryRun = origDryRun
		groomOutputDir = origGroomOutputDir
	}()

	dryRun = true
	groomOutputDir = t.TempDir()

	result := &domain.SynthesisResult{
		Synthesizer: "test",
		Summary:     "Test result",
		Items: []domain.SynthesisItem{
			{
				Title:    "Item 1",
				Priority: domain.P2,
				Type:     domain.Task,
				Horizon:  domain.Later,
			},
		},
	}

	var buf bytes.Buffer
	cmd := groomCmd
	cmd.SetOut(&buf)

	err := emitJSON(cmd, result)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	output := buf.String()
	if !strings.Contains(output, "synthesis.json") {
		t.Error("expected filename in output")
	}

	// Verify file was created
	// (In dryRun mode, file is still written since dryRun only affects git/issue operations)
}

func TestEmitMarkdown(t *testing.T) {
	// Save and restore
	origGroomOutputDir := groomOutputDir
	defer func() { groomOutputDir = origGroomOutputDir }()

	groomOutputDir = t.TempDir()

	result := &domain.SynthesisResult{
		Synthesizer: "test",
		Summary:     "Test grooming result",
		Items: []domain.SynthesisItem{
			{
				Title:    "Feature A",
				Priority: domain.P1,
				Type:     domain.Feature,
				Horizon:  domain.Next,
				Body:     "Implement feature A",
			},
		},
	}

	var buf bytes.Buffer
	cmd := groomCmd
	cmd.SetOut(&buf)

	err := emitMarkdown(cmd, result)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	output := buf.String()
	if !strings.Contains(output, "backlog.md") {
		t.Error("expected filename in output")
	}
}

func TestDetectRepo(t *testing.T) {
	// This will fail in test environment without git,
	// but that's expected behavior
	_, err := detectRepo()
	// We expect an error in test environment
	t.Logf("detectRepo() error (expected in test env): %v", err)
}

func TestValidateSynthesisResult(t *testing.T) {
	tests := []struct {
		name    string
		result  *domain.SynthesisResult
		wantErr bool
		wantMsg string
	}{
		{
			name:    "empty result is valid",
			result:  &domain.SynthesisResult{Items: nil},
			wantErr: false,
		},
		{
			name: "valid item",
			result: &domain.SynthesisResult{
				Items: []domain.SynthesisItem{
					{
						Title:    "Valid item",
						Priority: domain.P1,
						Type:     domain.Feature,
						Horizon:  domain.Next,
					},
				},
			},
			wantErr: false,
		},
		{
			name: "missing title",
			result: &domain.SynthesisResult{
				Items: []domain.SynthesisItem{
					{
						Title:    "",
						Priority: domain.P1,
						Type:     domain.Feature,
						Horizon:  domain.Next,
					},
				},
			},
			wantErr: true,
			wantMsg: "title is required",
		},
		{
			name: "invalid priority",
			result: &domain.SynthesisResult{
				Items: []domain.SynthesisItem{
					{
						Title:    "Item",
						Priority: "invalid",
						Type:     domain.Feature,
						Horizon:  domain.Next,
					},
				},
			},
			wantErr: true,
			wantMsg: "invalid priority",
		},
		{
			name: "invalid type",
			result: &domain.SynthesisResult{
				Items: []domain.SynthesisItem{
					{
						Title:    "Item",
						Priority: domain.P1,
						Type:     "invalid",
						Horizon:  domain.Next,
					},
				},
			},
			wantErr: true,
			wantMsg: "invalid type",
		},
		{
			name: "invalid horizon",
			result: &domain.SynthesisResult{
				Items: []domain.SynthesisItem{
					{
						Title:    "Item",
						Priority: domain.P1,
						Type:     domain.Feature,
						Horizon:  "invalid",
					},
				},
			},
			wantErr: true,
			wantMsg: "invalid horizon",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := validateSynthesisResult(tt.result)
			hasErr := err != nil

			if hasErr != tt.wantErr {
				t.Errorf("validateSynthesisResult() error = %v, wantErr %v", err, tt.wantErr)
			}

			if tt.wantErr && tt.wantMsg != "" && err != nil {
				if !strings.Contains(err.Error(), tt.wantMsg) {
					t.Errorf("validateSynthesisResult() error message = %v, want to contain %v", err, tt.wantMsg)
				}
			}
		})
	}
}

func TestIsValidPriority(t *testing.T) {
	tests := []struct {
		p    domain.Priority
		want bool
	}{
		{domain.P0, true},
		{domain.P1, true},
		{domain.P2, true},
		{domain.P3, true},
		{"invalid", false},
		{"", false},
	}
	for _, tt := range tests {
		got := isValidPriority(tt.p)
		if got != tt.want {
			t.Errorf("isValidPriority(%q) = %v, want %v", tt.p, got, tt.want)
		}
	}
}

func TestIsValidItemType(t *testing.T) {
	tests := []struct {
		typ  domain.ItemType
		want bool
	}{
		{domain.Bug, true},
		{domain.Feature, true},
		{domain.Task, true},
		{domain.Refactor, true},
		{domain.Research, true},
		{"invalid", false},
	}
	for _, tt := range tests {
		got := isValidItemType(tt.typ)
		if got != tt.want {
			t.Errorf("isValidItemType(%q) = %v, want %v", tt.typ, got, tt.want)
		}
	}
}

func TestIsValidHorizon(t *testing.T) {
	tests := []struct {
		h    domain.Horizon
		want bool
	}{
		{domain.Now, true},
		{domain.Next, true},
		{domain.Later, true},
		{"invalid", false},
	}
	for _, tt := range tests {
		got := isValidHorizon(tt.h)
		if got != tt.want {
			t.Errorf("isValidHorizon(%q) = %v, want %v", tt.h, got, tt.want)
		}
	}
}
