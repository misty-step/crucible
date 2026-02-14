package exec

import (
	"context"
	osexec "os/exec"
	"strings"
	"testing"
	"time"
)

func TestOSRunnerExecutesCommand(t *testing.T) {
	t.Parallel()

	runner := &OSRunner{}
	result, err := runner.Run(context.Background(), "go", []string{"version"}, RunOpts{})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result.ExitCode != 0 {
		t.Fatalf("got exit code %d, want 0", result.ExitCode)
	}
	if !strings.Contains(string(result.Stdout), "go version") {
		t.Fatalf("expected go version output, got %q", string(result.Stdout))
	}
}

func TestOSRunnerTimesOut(t *testing.T) {
	t.Parallel()

	if _, err := osexec.LookPath("sleep"); err != nil {
		t.Skip("sleep command not available; skip timeout test")
	}

	runner := &OSRunner{}
	_, err := runner.Run(context.Background(), "sleep", []string{"1"}, RunOpts{Timeout: 10 * time.Millisecond})
	if err == nil {
		t.Fatal("expected timeout error")
	}
	if !strings.Contains(err.Error(), "command timed out") {
		t.Fatalf("got %q, want timeout error", err)
	}
}
