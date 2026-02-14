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
// via Results (keyed by command name by default, or by command+args for
// per-invocation behavior). All calls are recorded in Calls.
type MockRunner struct {
	mu      sync.Mutex
	Results map[string]*RunResult // keyed by command or command+args
	Errors  map[string]error      // keyed by command or command+args
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

	m.Calls = append(m.Calls, MockCall{Name: name, Args: args, Opts: opts})

	key := mockCommandKey(name, args)
	if err, ok := m.Errors[key]; ok {
		result := m.Results[key] // may be nil
		return result, err
	}

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

func mockCommandKey(name string, args []string) string {
	return name + "\x00" + strings.Join(args, "\x00")
}
