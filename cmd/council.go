package cmd

import (
	"fmt"

	"github.com/spf13/cobra"
)

var councilCmd = &cobra.Command{
	Use:   "council",
	Short: "Run multi-model council for backlog grooming",
	Run: func(cmd *cobra.Command, args []string) {
		fmt.Println("not yet implemented")
	},
}

func init() {
	rootCmd.AddCommand(councilCmd)
}
