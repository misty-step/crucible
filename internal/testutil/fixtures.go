package testutil

import (
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/misty-step/crucible/internal/domain"
)

// testdataDir returns the absolute path to the project's testdata/ directory.
func testdataDir() string {
	_, file, _, _ := runtime.Caller(0)
	return filepath.Join(filepath.Dir(file), "..", "..", "testdata")
}

// LoadFixture reads a file from testdata/ and returns its contents.
func LoadFixture(t *testing.T, name string) []byte {
	t.Helper()
	path := filepath.Join(testdataDir(), name)
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("LoadFixture(%q): %v", name, err)
	}
	return data
}

// MustParseCouncilOutput loads and parses a council output fixture.
func MustParseCouncilOutput(t *testing.T, name string) *domain.CouncilOutput {
	t.Helper()
	data := LoadFixture(t, name)
	var out domain.CouncilOutput
	if err := json.Unmarshal(data, &out); err != nil {
		t.Fatalf("MustParseCouncilOutput(%q): %v", name, err)
	}
	return &out
}

// MustParseSynthesisResult loads and parses a synthesis result fixture.
func MustParseSynthesisResult(t *testing.T, name string) *domain.SynthesisResult {
	t.Helper()
	data := LoadFixture(t, name)
	var result domain.SynthesisResult
	if err := json.Unmarshal(data, &result); err != nil {
		t.Fatalf("MustParseSynthesisResult(%q): %v", name, err)
	}
	return &result
}
