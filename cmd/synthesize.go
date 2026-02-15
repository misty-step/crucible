package cmd

import (
	"fmt"

	"github.com/spf13/cobra"
)

var synthesizeCmd = &cobra.Command{
	Use:   "synthesize",
	Short: "Run synthesizer on council outputs",
	Run: func(_ *cobra.Command, _ []string) {
		fmt.Println("not yet implemented")
	},
}

func init() {
	rootCmd.AddCommand(synthesizeCmd)
}
