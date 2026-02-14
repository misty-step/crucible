package exec

import (
	"io"
	"strings"
)

// AllowedEnvKeys are the only environment variables passed to child processes.
var AllowedEnvKeys = []string{
	"HOME",
	"PATH",
	"OPENROUTER_API_KEY",
	"TMPDIR",
}

// SanitizeArg strips leading dashes to prevent argument injection in
// exec.Command calls where arguments come from untrusted model output.
// Also strips null bytes and trims whitespace.
func SanitizeArg(s string) string {
	s = strings.ReplaceAll(s, "\x00", "")
	s = strings.TrimSpace(s)
	s = strings.TrimLeft(s, "-")
	return s
}

// SanitizeTitle is SanitizeArg plus length enforcement for issue/PR titles.
func SanitizeTitle(s string, maxLen int) string {
	s = SanitizeArg(s)
	if len(s) > maxLen {
		s = s[:maxLen]
	}
	return s
}

// FilterEnv returns a slice of KEY=VALUE strings containing only allowed keys
// from the given environment. Use with exec.Cmd.Env to prevent leaking secrets.
func FilterEnv(env []string, allowed []string) []string {
	allowSet := make(map[string]bool, len(allowed))
	for _, key := range allowed {
		allowSet[key] = true
	}

	var filtered []string
	for _, entry := range env {
		key, _, ok := strings.Cut(entry, "=")
		if ok && allowSet[key] {
			filtered = append(filtered, entry)
		}
	}
	return filtered
}

// MaxModelOutputSize is the maximum bytes read from model output (1MB).
const MaxModelOutputSize = 1 << 20

// LimitedReader wraps an io.Reader with a size limit to prevent
// unbounded reads from model output.
func LimitedReader(r io.Reader) io.Reader {
	return io.LimitReader(r, MaxModelOutputSize)
}
