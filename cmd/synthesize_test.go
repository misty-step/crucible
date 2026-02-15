package cmd

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
)

func writeCouncilFixtures(t *testing.T, dir string) {
	t.Helper()

	outputs := []domain.CouncilOutput{
		{
			Councilor:   "STRATEGIST",
			Perspective: "product",
			Confidence:  0.85,
			Summary:     "Core pipeline needs completion",
			Items: []domain.CouncilItem{
				{
					Title:    "Implement end-to-end council execution",
					Priority: domain.P0,
					Type:     domain.Feature,
					Effort:   domain.Large,
				},
				{
					Title:    "Add progress indicators",
					Priority: domain.P2,
					Type:     domain.Feature,
					Effort:   domain.Small,
				},
			},
			Meta: domain.CouncilMeta{ItemsProposed: 2, ContextQuality: domain.MediumQuality},
		},
		{
			Councilor:   "ARCHITECT",
			Perspective: "engineering",
			Confidence:  0.9,
			Summary:     "Focus on testable infrastructure",
			Items: []domain.CouncilItem{
				{
					Title:    "Implement end-to-end council execution",
					Priority: domain.P0,
					Type:     domain.Feature,
					Effort:   domain.Large,
				},
				{
					Title:    "Add CommandRunner interface",
					Priority: domain.P0,
					Type:     domain.Task,
					Effort:   domain.Medium,
				},
			},
			Meta: domain.CouncilMeta{ItemsProposed: 2, ContextQuality: domain.HighQuality},
		},
	}

	for _, co := range outputs {
		data, err := json.MarshalIndent(co, "", "  ")
		if err != nil {
			t.Fatal(err)
		}
		filename := "council_" + co.Perspective + ".json"
		if err := os.WriteFile(filepath.Join(dir, filename), data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
}

func TestSynthesizeRequiresInputDir(t *testing.T) {
	root := rootCmd
	root.SetOut(new(bytes.Buffer))
	root.SetErr(new(bytes.Buffer))
	root.SetArgs([]string{"synthesize"})

	err := root.Execute()
	if err == nil {
		t.Fatal("expected error when --input-dir is missing")
	}
}

func TestSynthesizeToStdout(t *testing.T) {
	dir := t.TempDir()
	writeCouncilFixtures(t, dir)

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetArgs([]string{"synthesize", "--input-dir", dir})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	var result domain.SynthesisResult
	if err := json.Unmarshal(stdout.Bytes(), &result); err != nil {
		t.Fatalf("invalid JSON on stdout: %v\noutput: %s", err, stdout.String())
	}

	if result.Synthesizer != "PLACEHOLDER" {
		t.Errorf("got synthesizer %q, want PLACEHOLDER", result.Synthesizer)
	}
	if len(result.Items) == 0 {
		t.Fatal("expected at least one synthesis item")
	}

	if !strings.Contains(stderr.String(), "Synthesis complete") {
		t.Errorf("expected completion message on stderr, got: %s", stderr.String())
	}
}

func TestSynthesizeToFile(t *testing.T) {
	inputDir := t.TempDir()
	writeCouncilFixtures(t, inputDir)

	outputDir := t.TempDir()
	outputPath := filepath.Join(outputDir, "result.json")

	root := rootCmd
	root.SetOut(new(bytes.Buffer))
	root.SetErr(new(bytes.Buffer))
	root.SetArgs([]string{"synthesize", "--input-dir", inputDir, "--output", outputPath})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	data, err := os.ReadFile(outputPath)
	if err != nil {
		t.Fatalf("read output file: %v", err)
	}

	var result domain.SynthesisResult
	if err := json.Unmarshal(data, &result); err != nil {
		t.Fatalf("invalid JSON in output file: %v", err)
	}

	if len(result.Items) == 0 {
		t.Fatal("expected at least one synthesis item in output file")
	}
}

func TestSynthesizeEmptyDir(t *testing.T) {
	dir := t.TempDir()

	root := rootCmd
	root.SetOut(new(bytes.Buffer))
	root.SetErr(new(bytes.Buffer))
	root.SetArgs([]string{"synthesize", "--input-dir", dir})

	err := root.Execute()
	if err == nil {
		t.Fatal("expected error for empty input dir")
	}
	if !strings.Contains(err.Error(), "no council output") {
		t.Fatalf("expected 'no council output' error, got: %v", err)
	}
}

func TestSynthesizeBadInputDir(t *testing.T) {
	root := rootCmd
	root.SetOut(new(bytes.Buffer))
	root.SetErr(new(bytes.Buffer))
	root.SetArgs([]string{"synthesize", "--input-dir", "/nonexistent/dir"})

	err := root.Execute()
	if err == nil {
		t.Fatal("expected error for nonexistent dir")
	}
	if !strings.Contains(err.Error(), "read input dir") {
		t.Fatalf("expected read error, got: %v", err)
	}
}

func TestSynthesizeDeduplication(t *testing.T) {
	dir := t.TempDir()
	writeCouncilFixtures(t, dir)

	stdout := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(new(bytes.Buffer))
	root.SetArgs([]string{"synthesize", "--input-dir", dir, "--output", ""})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	var result domain.SynthesisResult
	if err := json.Unmarshal(stdout.Bytes(), &result); err != nil {
		t.Fatalf("invalid JSON: %v", err)
	}

	titleCount := 0
	for _, item := range result.Items {
		if item.Title == "Implement end-to-end council execution" {
			titleCount++
			if len(item.CouncilSupport.ProposedBy) < 2 {
				t.Errorf("expected 2 supporters for deduplicated item, got %d", len(item.CouncilSupport.ProposedBy))
			}
		}
	}
	if titleCount != 1 {
		t.Errorf("expected 1 deduplicated item, got %d", titleCount)
	}
}
