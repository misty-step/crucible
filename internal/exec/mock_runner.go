package exec

import (
	"context"
	"fmt"
	"strings"
	"sync"
)

// MockCall records a single invocation of Run.
type MockCall struct {
	Name string
	Args []string
	Opts RunOpts
}

// MockRunner is a test double for CommandRunner. Configure expected results
// via Results (keyed by exact invocation key or command name for fallback).
// All calls are recorded in Calls.
type MockRunner struct {
	mu      sync.Mutex
	Results map[string]*RunResult // keyed by invocation key or command name
	Errors  map[string]error      // keyed by invocation key or command name
	Calls   []MockCall
}

// NewMockRunner returns a MockRunner with initialized maps.
func NewMockRunner() *MockRunner {
	return &MockRunner{
		Results: make(map[string]*RunResult),
		Errors:  make(map[string]error),
	}
}

func (m *MockRunner) Run(_ context.Context, name string, args []string, opts RunOpts) (*RunResult, error) {
	m.mu.Lock()
	defer m.mu.Unlock()

	m.Calls = append(m.Calls, MockCall{
		Name: name,
		Args: append([]string(nil), args...),
		Opts: copyRunOpts(opts),
	})

	key := invocationKey(name, args)

	// Exact invocation match first (command + args)
	if err, ok := m.Errors[key]; ok {
		result := m.Results[key] // may be nil
		return result, err
	}

	// Fallback to command-name-only match
	if err, ok := m.Errors[name]; ok {
		result := m.Results[name] // may be nil
		return result, err
	}

	result, ok := m.Results[key]
	if !ok {
		result, ok = m.Results[name]
	}
	if !ok {
		return nil, fmt.Errorf("mock: no result configured for command %q args %q", name, strings.Join(args, " "))
	}

	return result, nil
}

func copyRunOpts(opts RunOpts) RunOpts {
	optsCopy := opts
	optsCopy.Env = append([]string(nil), opts.Env...)
	return optsCopy
}

func invocationKey(name string, args []string) string {
	if len(args) == 0 {
		return name
	}
	return name + "\x1f" + strings.Join(args, "\x1e")
}
