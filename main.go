package main

import (
	"flag"
	"fmt"
	"os"
)

var version = "v0.1.0"

func main() {
	versionFlag := flag.Bool("version", false, "print version")
	helpFlag := flag.Bool("help", false, "print help")
	flag.Parse()

	if *versionFlag {
		fmt.Printf("crucible %s\n", version)
		os.Exit(0)
	}

	if *helpFlag || len(os.Args) == 1 {
		fmt.Printf("crucible %s — multi-model backlog grooming CLI\n\n", version)
		fmt.Println("Usage: crucible [flags]")
		fmt.Println("\nFlags:")
		flag.PrintDefaults()
		fmt.Println("\nSubcommands (coming soon):")
		fmt.Println("  council     Spawn multi-model council for backlog grooming")
		fmt.Println("  synthesize  Run synthesizer to evaluate council outputs")
		fmt.Println("  issues      Create GitHub issues from prioritized backlog")
		os.Exit(0)
	}

	// TODO: Implement council, synthesizer, and issue-creation subsystems
	fmt.Println("crucible: not yet implemented. Run with --help for usage.")
}
