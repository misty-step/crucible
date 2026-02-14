package cmd

import "fmt"

// RootCmd represents the root command for crucible
var RootCmd = &Command{
	Use:   "crucible",
	Short: "Multi-model backlog grooming CLI",
	Long: `Crucible transforms raw ideas into prioritized, actionable work
through a multi-model council approach.`,
}

// Command represents a CLI command (placeholder for future cobra migration)
type Command struct {
	Use   string
	Short string
	Long  string
}

// Execute runs the command (placeholder)
func (c *Command) Execute() error {
	fmt.Println(c.Long)
	return nil
}
