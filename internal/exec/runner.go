package exec

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"os/exec"
	"time"
)

// CommandRunner executes external commands. All components that shell out
// depend on this interface, enabling deterministic tests via MockRunner.
type CommandRunner interface {
	Run(ctx context.Context, name string, args []string, opts RunOpts) (*RunResult, error)
}

// RunOpts configures a command execution.
type RunOpts struct {
	Stdin   io.Reader
	Env     []string      // explicit env vars (not inherited)
	Dir     string        // working directory
	Timeout time.Duration // 0 means no additional timeout beyond ctx
}

// RunResult captures the output of a completed command.
type RunResult struct {
	Stdout   []byte
	Stderr   []byte
	ExitCode int
	Duration time.Duration
}

// OSRunner executes commands via os/exec.
type OSRunner struct{}

func (r *OSRunner) Run(ctx context.Context, name string, args []string, opts RunOpts) (*RunResult, error) {
	if opts.Timeout > 0 {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, opts.Timeout)
		defer cancel()
	}

	cmd := exec.CommandContext(ctx, name, args...)
	if opts.Dir != "" {
		cmd.Dir = opts.Dir
	}
	if opts.Env != nil {
		cmd.Env = opts.Env
	}
	if opts.Stdin != nil {
		cmd.Stdin = opts.Stdin
	}

	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	start := time.Now()
	err := cmd.Run()
	duration := time.Since(start)

	result := &RunResult{
		Stdout:   stdout.Bytes(),
		Stderr:   stderr.Bytes(),
		Duration: duration,
	}

	if cmd.ProcessState != nil {
		result.ExitCode = cmd.ProcessState.ExitCode()
	}

	if err != nil {
		if ctx.Err() == context.DeadlineExceeded {
			return result, fmt.Errorf("command timed out: %s %v", name, args)
		}
		return result, err
	}

	return result, nil
}
