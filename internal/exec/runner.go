package exec

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
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

// limitedBuffer stores up to a fixed max of captured bytes.
type limitedBuffer struct {
	buf      bytes.Buffer
	limit    int
	exceeded bool
}

func (b *limitedBuffer) Write(p []byte) (int, error) {
	if b.limit <= 0 {
		return len(p), nil
	}

	n := len(p)
	remaining := b.limit - b.buf.Len()
	if remaining <= 0 {
		b.exceeded = true
		return n, nil
	}

	if n > remaining {
		n = remaining
		b.exceeded = true
	}

	_, _ = b.buf.Write(p[:n])
	return len(p), nil
}

func (b *limitedBuffer) Bytes() []byte {
	return b.buf.Bytes()
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
	} else {
		cmd.Env = FilterEnv(os.Environ(), AllowedEnvKeys)
	}
	if opts.Stdin != nil {
		cmd.Stdin = opts.Stdin
	}

	stdout := &limitedBuffer{limit: MaxModelOutputSize}
	stderr := &limitedBuffer{limit: MaxModelOutputSize}
	cmd.Stdout = stdout
	cmd.Stderr = stderr

	start := time.Now()
	err := cmd.Run()
	duration := time.Since(start)

	result := &RunResult{
		Stdout:   append([]byte(nil), stdout.Bytes()...),
		Stderr:   append([]byte(nil), stderr.Bytes()...),
		Duration: duration,
	}

	if cmd.ProcessState != nil {
		result.ExitCode = cmd.ProcessState.ExitCode()
	} else if err != nil {
		result.ExitCode = 1
	}

	if err != nil {
		if ctx.Err() != nil {
			if errors.Is(ctx.Err(), context.DeadlineExceeded) {
				return result, fmt.Errorf("command timed out: %s %v: %w", name, args, err)
			}
			if errors.Is(ctx.Err(), context.Canceled) {
				return result, fmt.Errorf("command canceled: %s %v: %w", name, args, err)
			}
		}
		// Non-zero exit is captured in result.ExitCode, not propagated as error.
		// Only return errors for failures to execute (not found, permission, etc).
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) {
			return result, nil
		}
		return result, err
	}

	return result, nil
}
