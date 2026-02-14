package reposcanner

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	domain "github.com/misty-step/crucible/internal/domain"
	cruxexec "github.com/misty-step/crucible/internal/exec"
)

// Gatherer collects repository context for council input.
type Gatherer interface {
	Gather(ctx context.Context) (*RepoContext, error)
}

// RepoContext holds all gathered repository state.
type RepoContext struct {
	domain.RepoState
	Vision string
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
const defaultVisionPath = "VISION.md"
const maxFileTreeEntries = 300

var errFileTreeLimitReached = errors.New("file tree entry limit reached")

func (g *CLIGatherer) ensureRunner() cruxexec.CommandRunner {
	if g.Runner != nil {
		return g.Runner
	}

	g.Runner = &cruxexec.OSRunner{}
	return g.Runner
}

func (g *CLIGatherer) Gather(ctx context.Context) (*RepoContext, error) {
	g.ensureRunner()

	var commitResult, issueResult, prResult []string
	var fileTree string

	var wg sync.WaitGroup
	wg.Add(4)

	go func() {
		defer wg.Done()
		commitResult = g.runLines(ctx, "git", []string{"log", "--oneline", "-20"})
	}()

	go func() {
		defer wg.Done()
		issueResult = g.runLines(ctx, "gh", []string{"issue", "list", "--state", "open", "--limit", "30", "--json", "number,title", "--jq", `.[] | "#\(.number) \(.title)"`})
	}()

	go func() {
		defer wg.Done()
		prResult = g.runLines(ctx, "gh", []string{"pr", "list", "--state", "open", "--limit", "20", "--json", "number,title", "--jq", `.[] | "#\(.number) \(.title)"`})
	}()

	go func() {
		defer wg.Done()
		fileTree = g.getFileTree()
	}()

	wg.Wait()

	visionPath := g.VisionPath
	if visionPath == "" {
		visionPath = defaultVisionPath
	}
	if g.Dir != "" && !filepath.IsAbs(visionPath) {
		visionPath = filepath.Join(g.Dir, visionPath)
	}

	rc := &RepoContext{
		RepoState: domain.RepoState{
			RecentCommits: commitResult,
			OpenIssues:    issueResult,
			OpenPRs:       prResult,
			FileTree:      fileTree,
		},
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

// getFileTree returns a deterministic file list rooted at g.Dir.
func (g *CLIGatherer) getFileTree() string {
	root := g.Dir
	if root == "" {
		root = "."
	}

	entries := make([]string, 0)

	err := filepath.WalkDir(root, func(path string, d fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return nil
		}

		if d.IsDir() {
			if d.Name() == ".git" || d.Name() == "vendor" {
				return filepath.SkipDir
			}
			return nil
		}

		rel, relErr := filepath.Rel(root, path)
		if relErr != nil {
			return relErr
		}
		if rel == "." {
			return nil
		}

		entries = append(entries, "./"+filepath.ToSlash(rel))

		if len(entries) >= maxFileTreeEntries {
			return errFileTreeLimitReached
		}

		return nil
	})

	if err != nil && !errors.Is(err, errFileTreeLimitReached) {
		return ""
	}

	sort.Strings(entries)
	return strings.Join(entries, "\n")
}
