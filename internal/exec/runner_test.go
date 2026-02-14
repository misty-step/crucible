package exec

import (
	"context"
	osexec "os/exec"
	"strings"
	"testing"
	"time"
)

func TestMockRunnerRecordsCalls(t *testing.T) {
	t.Parallel()

	m := NewMockRunner()
	m.Results[invocationKey("echo", []string{"hello"})] = &RunResult{Stdout: []byte("hello\n")}

	result, err := m.Run(context.Background(), "echo", []string{"hello"}, RunOpts{})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if string(result.Stdout) != "hello\n" {
		t.Fatalf("got stdout %q, want %q", result.Stdout, "hello\n")
	}
	if len(m.Calls) != 1 {
		t.Fatalf("got %d calls, want 1", len(m.Calls))
	}
	if m.Calls[0].Name != "echo" {
		t.Fatalf("got call name %q, want %q", m.Calls[0].Name, "echo")
	}
}

func TestMockRunnerInvocationKeying(t *testing.T) {
	t.Parallel()

	m := NewMockRunner()
	m.Results[invocationKey("echo", []string{"hello"})] = &RunResult{Stdout: []byte("hello\n")}
	m.Results[invocationKey("echo", []string{"world"})] = &RunResult{Stdout: []byte("world\n")}

	if _, err := m.Run(context.Background(), "echo", []string{"hello"}, RunOpts{}); err != nil {
		t.Fatalf("unexpected error for hello: %v", err)
	}
	if _, err := m.Run(context.Background(), "echo", []string{"world"}, RunOpts{}); err != nil {
		t.Fatalf("unexpected error for world: %v", err)
	}

	if _, err := m.Run(context.Background(), "echo", nil, RunOpts{}); err == nil {
		t.Fatal("expected error for unspecified invocation")
	}
}

func TestMockRunnerRecordsArgsByValue(t *testing.T) {
	t.Parallel()

	args := []string{"hello"}
	m := NewMockRunner()
	m.Results[invocationKey("echo", args)] = &RunResult{Stdout: []byte("hello\n")}

	if _, err := m.Run(context.Background(), "echo", args, RunOpts{}); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	args[0] = "modified"
	if got := m.Calls[0].Args[0]; got != "hello" {
		t.Fatalf("got %q call arg, want %q", got, "hello")
	}
}

func TestMockRunnerReturnsError(t *testing.T) {
	t.Parallel()

	m := NewMockRunner()
	m.Results["fail"] = &RunResult{Stderr: []byte("boom"), ExitCode: 1}
	m.Errors["fail"] = &execError{msg: "exit status 1"}

	result, err := m.Run(context.Background(), "fail", nil, RunOpts{})
	if err == nil {
		t.Fatal("expected error, got nil")
	}
	if result == nil {
		t.Fatal("expected result even on error")
	}
	if result.ExitCode != 1 {
		t.Fatalf("got exit code %d, want 1", result.ExitCode)
	}
}

func TestMockRunnerUnconfiguredCommand(t *testing.T) {
	t.Parallel()

	m := NewMockRunner()
	_, err := m.Run(context.Background(), "unknown", nil, RunOpts{})
	if err == nil {
		t.Fatal("expected error for unconfigured command")
	}
	if !strings.Contains(err.Error(), "no result configured") {
		t.Fatalf("got error %q, want 'no result configured'", err)
	}
}

func TestOSRunnerEcho(t *testing.T) {
	t.Parallel()
	requireCommand(t, "echo")

	r := &OSRunner{}
	result, err := r.Run(context.Background(), "echo", []string{"test"}, RunOpts{})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got := strings.TrimSpace(string(result.Stdout)); got != "test" {
		t.Fatalf("got stdout %q, want %q", got, "test")
	}
	if result.ExitCode != 0 {
		t.Fatalf("got exit code %d, want 0", result.ExitCode)
	}
}

func TestOSRunnerTimeout(t *testing.T) {
	t.Parallel()
	requireCommand(t, "sleep")

	r := &OSRunner{}
	_, err := r.Run(context.Background(), "sleep", []string{"10"}, RunOpts{
		Timeout: 50 * time.Millisecond,
	})
	if err == nil {
		t.Fatal("expected timeout error")
	}
	if !strings.Contains(err.Error(), "timed out") {
		t.Fatalf("got error %q, want timeout", err)
	}
}

func TestOSRunnerStdin(t *testing.T) {
	t.Parallel()
	requireCommand(t, "cat")

	r := &OSRunner{}
	result, err := r.Run(context.Background(), "cat", nil, RunOpts{
		Stdin: strings.NewReader("hello from stdin"),
	})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if string(result.Stdout) != "hello from stdin" {
		t.Fatalf("got stdout %q, want %q", result.Stdout, "hello from stdin")
	}
}

// execError is a simple error type for tests.
type execError struct{ msg string }

func (e *execError) Error() string { return e.msg }

func requireCommand(t *testing.T, name string) {
	t.Helper()
	if _, err := osexec.LookPath(name); err != nil {
		t.Skipf("command %q not found in PATH", name)
	}
}
