package reposcanner

import (
	"context"
	"fmt"
	"os"
	"strings"
	"time"

	cruxexec "github.com/misty-step/crucible/internal/exec"
)

// Gatherer collects repository context for council input.
type Gatherer interface {
	Gather(ctx context.Context) (*RepoContext, error)
}

// RepoContext holds all gathered repository state.
type RepoContext struct {
	RecentCommits []string
	OpenIssues    []string
	OpenPRs       []string
	FileTree      string
	Vision        string
}

// Render formats the context as a markdown section for prompt injection.
func (rc *RepoContext) Render() string {
	var b strings.Builder

	b.WriteString("## Repository Context\n\n")

	if len(rc.RecentCommits) > 0 {
		b.WriteString("### Recent Commits\n")
		for _, c := range rc.RecentCommits {
			fmt.Fprintf(&b, "- %s\n", c)
		}
		b.WriteString("\n")
	}

	if len(rc.OpenIssues) > 0 {
		b.WriteString("### Open Issues\n")
		for _, i := range rc.OpenIssues {
			fmt.Fprintf(&b, "- %s\n", i)
		}
		b.WriteString("\n")
	}

	if len(rc.OpenPRs) > 0 {
		b.WriteString("### Open PRs\n")
		for _, pr := range rc.OpenPRs {
			fmt.Fprintf(&b, "- %s\n", pr)
		}
		b.WriteString("\n")
	}

	if rc.FileTree != "" {
		b.WriteString("### File Tree\n```\n")
		b.WriteString(rc.FileTree)
		b.WriteString("\n```\n\n")
	}

	if rc.Vision != "" {
		b.WriteString("### Vision\n")
		b.WriteString(rc.Vision)
		b.WriteString("\n")
	}

	return b.String()
}

// CLIGatherer gathers context by shelling out to git and gh.
type CLIGatherer struct {
	Runner     cruxexec.CommandRunner
	VisionPath string
	Dir        string
}

const cmdTimeout = 10 * time.Second

func (g *CLIGatherer) Gather(ctx context.Context) (*RepoContext, error) {
	rc := &RepoContext{}

	rc.RecentCommits = g.runLines(ctx, "git", []string{"log", "--oneline", "-20"})
	rc.OpenIssues = g.runLines(ctx, "gh", []string{"issue", "list", "--state", "open", "--limit", "30", "--json", "number,title", "--jq", `.[] | "#\(.number) \(.title)"`})
	rc.OpenPRs = g.runLines(ctx, "gh", []string{"pr", "list", "--state", "open", "--limit", "20", "--json", "number,title", "--jq", `.[] | "#\(.number) \(.title)"`})
	rc.FileTree = g.runString(ctx, "find", []string{".", "-type", "f", "-not", "-path", "./.git/*", "-not", "-path", "./vendor/*"})

	visionPath := g.VisionPath
	if visionPath == "" {
		visionPath = "VISION.md"
	}
	if data, err := os.ReadFile(visionPath); err == nil {
		rc.Vision = string(data)
	}

	return rc, nil
}

// runLines executes a command and returns stdout split into non-empty lines.
// Returns nil on error (graceful fallback).
func (g *CLIGatherer) runLines(ctx context.Context, name string, args []string) []string {
	out := g.runString(ctx, name, args)
	if out == "" {
		return nil
	}
	lines := strings.Split(out, "\n")
	var result []string
	for _, line := range lines {
		if trimmed := strings.TrimSpace(line); trimmed != "" {
			result = append(result, trimmed)
		}
	}
	return result
}

// runString executes a command and returns stdout as a string.
// Returns "" on error (graceful fallback).
func (g *CLIGatherer) runString(ctx context.Context, name string, args []string) string {
	result, err := g.Runner.Run(ctx, name, args, cruxexec.RunOpts{
		Dir:     g.Dir,
		Timeout: cmdTimeout,
	})
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(result.Stdout))
}
