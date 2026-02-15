package cmd

import (
	"fmt"

	"github.com/spf13/cobra"
)

var createIssuesCmd = &cobra.Command{
	Use:   "create-issues",
	Short: "Create GitHub issues from synthesizer output",
	Run: func(_ *cobra.Command, args []string) {
		fmt.Println("not yet implemented")
	},
}

func init() {
	rootCmd.AddCommand(createIssuesCmd)
}
