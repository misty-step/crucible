package council

import (
	"context"
	"encoding/json"
	"fmt"
	"testing"

	cruxexec "github.com/misty-step/crucible/internal/exec"
	"github.com/misty-step/crucible/internal/models"
)

func TestExtractJSONWithBracesInsideStrings(t *testing.T) {
	t.Parallel()

	raw := []byte("noise before\n{\"note\":\"ignore { braces } inside\",\"nested\":{\"value\":\"{escaped}\"}}\nnoise after")
	got := ExtractJSON(raw)
	if got == nil {
		t.Fatal("expected JSON bytes")
	}

	want := "{\"note\":\"ignore { braces } inside\",\"nested\":{\"value\":\"{escaped}\"}}"
	if string(got) != want {
		t.Fatalf("got %q, want %q", string(got), want)
	}
}

func TestIsPermanentError(t *testing.T) {
	t.Parallel()

	wrapped := fmt.Errorf("outer: %w", &permanentError{fmt.Errorf("auth failure")})
	if !isPermanentError(wrapped) {
		t.Fatal("expected permanent error to be detected via wrapped type")
	}

	if isPermanentError(fmt.Errorf("not permanent")) {
		t.Fatal("expected non-permanent error")
	}
}

func TestInvokeAgentSanitizesPrompt(t *testing.T) {
	t.Parallel()

	mock := cruxexec.NewMockRunner()
	spawner := &Spawner{
		Runner: mock,
		Env:    []string{"HOME=/tmp"},
	}

	output := map[string]interface{}{
		"councilor": "STRATEGIST",
		"perspective": "product",
		"confidence": 0.9,
		"summary": "ok",
	}
	stdout, _ := json.Marshal(output)

	mock.Results["opencode"] = &cruxexec.RunResult{
		Stdout:   stdout,
		ExitCode: 0,
	}

	_, err := spawner.invokeAgent(context.Background(), "product", models.Model{ID: "moonshotai/kimi-k2.5"}, " --payload\u0000value ", 0)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(mock.Calls) != 1 {
		t.Fatalf("expected 1 runner call, got %d", len(mock.Calls))
	}

	args := mock.Calls[0].Args
	got := args[len(args)-1]
	want := "payloadvalue"
	if got != want {
		t.Fatalf("got sanitized prompt %q, want %q", got, want)
	}
}
