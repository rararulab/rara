// commands.go defines CLI subcommands for worktree management.
package worktree

import (
	"context"
	"fmt"
	"os"
	"os/exec"

	"github.com/urfave/cli/v3"
)

// Cmd returns the top-level "worktree" command group.
func Cmd() *cli.Command {
	return &cli.Command{
		Name:    "worktree",
		Aliases: []string{"wt"},
		Usage:   "Manage git worktree lifecycle",
		// Default action: launch interactive TUI
		Action: func(_ context.Context, _ *cli.Command) error {
			return RunTUI()
		},
		Commands: []*cli.Command{
			{
				Name:    "list",
				Aliases: []string{"ls"},
				Usage:   "List all worktrees (non-interactive)",
				Action: func(_ context.Context, _ *cli.Command) error {
					return runList()
				},
			},
			{
				Name:  "clean",
				Usage: "Remove worktrees whose branches are merged into main (non-interactive)",
				Action: func(_ context.Context, _ *cli.Command) error {
					return runClean()
				},
			},
			{
				Name:  "nuke",
				Usage: "Force-remove ALL worktrees except the main checkout (non-interactive)",
				Action: func(_ context.Context, _ *cli.Command) error {
					return runNuke()
				},
			},
		},
	}
}

func runList() error {
	out, err := exec.Command("git", "worktree", "list").CombinedOutput()
	if err != nil {
		return fmt.Errorf("git worktree list: %w\n%s", err, out)
	}
	fmt.Print(string(out))
	return nil
}

func runClean() error {
	if err := Prune(); err != nil {
		return err
	}

	mainPath, err := MainPath()
	if err != nil {
		return err
	}

	merged, err := MergedBranches()
	if err != nil {
		return err
	}
	if len(merged) == 0 {
		fmt.Println("✅ No merged branches to clean up.")
		return nil
	}

	entries, err := List()
	if err != nil {
		return err
	}

	branchHandled := make(map[string]bool)
	removed := 0

	for _, e := range entries {
		if e.Path == mainPath || e.Branch == "" || !merged[e.Branch] {
			continue
		}
		fmt.Printf("🗑️  Removing worktree: %s (branch: %s)\n", e.Path, e.Branch)
		if err := Remove(e.Path, false); err != nil {
			fmt.Fprintf(os.Stderr, "  ⚠️  %s\n", err)
			continue
		}
		if err := DeleteBranch(e.Branch, false); err != nil {
			fmt.Fprintf(os.Stderr, "  ⚠️  %s\n", err)
		}
		branchHandled[e.Branch] = true
		removed++
	}

	for branch := range merged {
		if branchHandled[branch] {
			continue
		}
		fmt.Printf("🗑️  Deleting merged branch: %s (no worktree)\n", branch)
		if err := DeleteBranch(branch, false); err != nil {
			fmt.Fprintf(os.Stderr, "  ⚠️  %s\n", err)
			continue
		}
		removed++
	}

	fmt.Printf("✅ Cleaned up %d merged worktree(s)/branch(es).\n", removed)
	return nil
}

func runNuke() error {
	mainPath, err := MainPath()
	if err != nil {
		return err
	}

	entries, err := List()
	if err != nil {
		return err
	}

	removed := 0
	for _, e := range entries {
		if e.Path == mainPath {
			continue
		}
		fmt.Printf("🗑️  Removing: %s\n", e.Path)
		if err := Remove(e.Path, true); err != nil {
			fmt.Fprintf(os.Stderr, "  ⚠️  %s — cleaning up manually\n", err)
			os.RemoveAll(e.Path)
		}
		if e.Branch != "" {
			_ = DeleteBranch(e.Branch, true)
		}
		removed++
	}

	_ = Prune()
	fmt.Printf("✅ Removed %d worktree(s).\n", removed)
	return nil
}
