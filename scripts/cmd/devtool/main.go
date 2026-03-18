// Entry point for the devtool CLI — a unified developer toolkit for rara.
package main

import (
	"context"
	"log"
	"os"

	"github.com/rararulab/rara/scripts/internal/agentmd"
	"github.com/rararulab/rara/scripts/internal/deps"
	"github.com/rararulab/rara/scripts/internal/worktree"
	"github.com/urfave/cli/v3"
)

func main() {
	cmd := &cli.Command{
		Name:  "devtool",
		Usage: "Unified developer toolkit for rara",
		Commands: []*cli.Command{
			agentmd.Cmd(),
			worktree.Cmd(),
			deps.Cmd(),
		},
	}

	if err := cmd.Run(context.Background(), os.Args); err != nil {
		log.Fatal(err)
	}
}
