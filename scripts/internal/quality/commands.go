// Package quality implements the `devtool quality-matrix` subcommand.
// It scans all crates in the workspace and generates a markdown quality report.
package quality

import (
	"context"
	"fmt"
	"os"

	"github.com/urfave/cli/v3"
)

// Cmd returns the top-level "quality-matrix" command.
func Cmd() *cli.Command {
	return &cli.Command{
		Name:  "quality-matrix",
		Usage: "Scan all crates and generate a quality matrix markdown report",
		Flags: []cli.Flag{
			&cli.StringFlag{
				Name:  "output",
				Usage: "Write output to file instead of stdout",
			},
		},
		Action: func(_ context.Context, cmd *cli.Command) error {
			root, err := findRepoRoot()
			if err != nil {
				return err
			}

			crates, err := discoverCrates(root)
			if err != nil {
				return fmt.Errorf("discovering crates: %w", err)
			}

			md := renderMarkdown(crates)

			outPath := cmd.String("output")
			if outPath != "" {
				if err := os.WriteFile(outPath, []byte(md), 0644); err != nil {
					return fmt.Errorf("writing output file: %w", err)
				}
				fmt.Fprintf(os.Stderr, "Wrote quality matrix to %s\n", outPath)
				return nil
			}

			fmt.Print(md)
			return nil
		},
	}
}
