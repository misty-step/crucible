package exec

import (
	"io"
	"strings"
	"testing"
)

func TestSanitizeArg(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name  string
		input string
		want  string
	}{
		{"clean", "hello", "hello"},
		{"leading_dashes", "--repo evil/repo", "repo evil/repo"},
		{"single_dash", "-flag", "flag"},
		{"many_dashes", "---triple", "triple"},
		{"null_bytes", "hello\x00world", "helloworld"},
		{"whitespace", "  padded  ", "padded"},
		{"dash_and_space", " --flag ", "flag"},
		{"empty", "", ""},
		{"only_dashes", "---", ""},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := SanitizeArg(tc.input); got != tc.want {
				t.Fatalf("SanitizeArg(%q) = %q, want %q", tc.input, got, tc.want)
			}
		})
	}
}

func TestSanitizeTitle(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name   string
		input  string
		maxLen int
		want   string
	}{
		{"short", "Fix bug", 200, "Fix bug"},
		{"truncated", "A very long title that exceeds the limit", 10, "A very lon"},
		{"with_dashes", "--malicious title", 200, "malicious title"},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := SanitizeTitle(tc.input, tc.maxLen); got != tc.want {
				t.Fatalf("SanitizeTitle(%q, %d) = %q, want %q", tc.input, tc.maxLen, got, tc.want)
			}
		})
	}
}

func TestFilterEnv(t *testing.T) {
	t.Parallel()

	env := []string{
		"HOME=/Users/test",
		"PATH=/usr/bin",
		"OPENROUTER_API_KEY=sk-test",
		"SECRET_TOKEN=should-not-appear",
		"AWS_SECRET_KEY=also-hidden",
	}

	got := FilterEnv(env, AllowedEnvKeys)

	if len(got) != 3 {
		t.Fatalf("got %d env vars, want 3: %v", len(got), got)
	}

	// Verify SECRET_TOKEN and AWS_SECRET_KEY are filtered
	for _, entry := range got {
		if strings.HasPrefix(entry, "SECRET_TOKEN") || strings.HasPrefix(entry, "AWS_SECRET_KEY") {
			t.Fatalf("leaked secret env var: %s", entry)
		}
	}
}

func TestFilterEnvEmpty(t *testing.T) {
	t.Parallel()

	got := FilterEnv(nil, AllowedEnvKeys)
	if got != nil {
		t.Fatalf("expected nil, got %v", got)
	}
}

func TestLimitedReader(t *testing.T) {
	t.Parallel()

	// Create a reader larger than the limit
	large := strings.NewReader(strings.Repeat("x", MaxModelOutputSize+100))
	limited := LimitedReader(large)

	data, err := io.ReadAll(limited)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(data) != MaxModelOutputSize {
		t.Fatalf("got %d bytes, want %d", len(data), MaxModelOutputSize)
	}
}
