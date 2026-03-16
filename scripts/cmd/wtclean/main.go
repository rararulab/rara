// Entry point for the wt-clean CLI tool.
package main

import (
	"context"
	"log"
	"os"

	"github.com/rararulab/rara/scripts/internal/worktree"
	"github.com/urfave/cli/v3"
)

func main() {
	cmd := &cli.Command{
		Name:  "wt-clean",
		Usage: "Manage git worktree lifecycle",
		Commands: []*cli.Command{
			worktree.ListCmd(),
			worktree.CleanCmd(),
			worktree.NukeCmd(),
		},
		DefaultCommand: "list",
	}

	if err := cmd.Run(context.Background(), os.Args); err != nil {
		log.Fatal(err)
	}
}
