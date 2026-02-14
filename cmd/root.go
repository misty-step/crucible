package cmd

import (
	"os"

	"github.com/spf13/cobra"
)

var version = "dev"

var (
	verbose bool
	vision  string
	dryRun  bool
)

var rootCmd = &cobra.Command{
	Use:   "crucible",
	Short: "Multi-model backlog grooming CLI",
	RunE: func(cmd *cobra.Command, args []string) error {
		return cmd.Help()
	},
}

func Execute() {
	if err := rootCmd.Execute(); err != nil {
		os.Exit(1)
	}
}

func init() {
	rootCmd.Version = version
	rootCmd.PersistentFlags().BoolVar(&verbose, "verbose", false, "verbose output")
	rootCmd.PersistentFlags().StringVar(&vision, "vision", "VISION.md", "path to vision file")
	rootCmd.PersistentFlags().BoolVar(&dryRun, "dry-run", false, "dry run")
}
