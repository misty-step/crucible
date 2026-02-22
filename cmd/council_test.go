package cmd

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
	"github.com/misty-step/crucible/internal/telemetry"
)

func TestCouncilRequiresRepo(t *testing.T) {
	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetArgs([]string{"council"})

	err := root.Execute()
	if err == nil {
		t.Fatal("expected error when --repo is missing")
	}
}

func TestCouncilDryRun(t *testing.T) {
	repoDir := t.TempDir()

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetArgs([]string{"council", "--repo", repoDir, "--dry-run"})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stdout.String(), "dry-run") {
		t.Fatalf("expected dry-run output, got: %s", stdout.String())
	}
}

func TestCouncilDryRunVerbose(t *testing.T) {
	repoDir := t.TempDir()

	if err := os.WriteFile(filepath.Join(repoDir, "VISION.md"), []byte("Test vision"), 0o644); err != nil {
		t.Fatal(err)
	}

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetArgs([]string{"council", "--repo", repoDir, "--dry-run", "--verbose", "--vision", filepath.Join(repoDir, "VISION.md")})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "Gathering repository context") {
		t.Errorf("expected verbose output on stderr, got: %s", stderr.String())
	}
}

func TestCouncilInteractive(t *testing.T) {
	repoDir := t.TempDir()
	visionPath := filepath.Join(repoDir, "VISION.md")

	if err := os.WriteFile(visionPath, []byte("Test vision content"), 0o644); err != nil {
		t.Fatal(err)
	}

	// Simulate interactive input from user
	input := "Prioritize reliability\nFocus on testing\n\n"
	stdin := strings.NewReader(input)

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetIn(stdin)
	root.SetArgs([]string{"council", "--repo", repoDir, "--dry-run", "--interactive", "--vision", visionPath})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	output := stdout.String()

	// Verify context summary is printed
	if !strings.Contains(output, "Repository Context Summary") {
		t.Errorf("expected context summary, got: %s", output)
	}

	// Verify vision is shown
	if !strings.Contains(output, "Vision:") {
		t.Errorf("expected vision header in output, got: %s", output)
	}

	// Verify human input prompt appears
	if !strings.Contains(output, "What priorities, concerns, or ideas") {
		t.Errorf("expected human input prompt, got: %s", output)
	}

	// Verify dry-run completes
	if !strings.Contains(output, "dry-run") {
		t.Errorf("expected dry-run output, got: %s", output)
	}

	t.Logf("stdout: %s", output)
	t.Logf("stderr: %s", stderr.String())
}

func TestCouncilInteractiveMultiLine(t *testing.T) {
	repoDir := t.TempDir()

	// Simulate multi-line input ending with empty line
	input := "Line one\nLine two\nLine three\n\n"
	stdin := strings.NewReader(input)

	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetIn(stdin)
	root.SetArgs([]string{"council", "--repo", repoDir, "--dry-run", "--interactive"})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Verify council completes
	if !strings.Contains(stdout.String(), "dry-run") {
		t.Errorf("expected dry-run output, got: %s", stdout.String())
	}
}

func TestTelemetryReportGenerated(t *testing.T) {
	repoDir := t.TempDir()

	// Create mock council output
	councilDir := filepath.Join(repoDir, "council-output")
	if err := os.MkdirAll(councilDir, 0755); err != nil {
		t.Fatal(err)
	}

	output := domain.CouncilOutput{
		Councilor:   "test-councilor",
		Perspective: "product",
		Confidence:  0.9,
		Summary:     "Test summary",
		Items: []domain.CouncilItem{
			{Title: "Test item", Priority: domain.P2},
		},
	}

	data, _ := json.Marshal(output)
	if err := os.WriteFile(filepath.Join(councilDir, "council_product.json"), data, 0644); err != nil {
		t.Fatal(err)
	}

	// Run synthesize
	stdout := new(bytes.Buffer)
	stderr := new(bytes.Buffer)

	root := rootCmd
	root.SetOut(stdout)
	root.SetErr(stderr)
	root.SetArgs([]string{"synthesize", "--input-dir", councilDir, "--verbose"})

	err := root.Execute()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Verify telemetry was written
	runsDir := filepath.Join(".", ".crucible", "runs")
	entries, err := os.ReadDir(runsDir)
	if err != nil {
		t.Fatalf("failed to read runs directory: %v", err)
	}

	foundReport := false
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), "run_") && strings.HasSuffix(entry.Name(), ".json") {
			foundReport = true

			// Verify report structure
			data, err := os.ReadFile(filepath.Join(runsDir, entry.Name()))
			if err != nil {
				t.Fatalf("failed to read report: %v", err)
			}

			var report telemetry.RunReport
			if err := json.Unmarshal(data, &report); err != nil {
				t.Fatalf("failed to unmarshal report: %v", err)
			}

			if report.RunID == "" {
				t.Error("RunID should not be empty")
			}
			if report.Timestamp == "" {
				t.Error("Timestamp should not be empty")
			}

			break
		}
	}

	if !foundReport {
		t.Error("No telemetry report found")
	}

	// Cleanup
	_ = os.RemoveAll(runsDir)
}
