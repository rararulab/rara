// wt-clean manages git worktree lifecycle.
//
// Usage:
//
//	go run scripts/wt-clean.go [command]
//
// Commands:
//
//	list   — list all worktrees (default)
//	clean  — remove worktrees whose branches are merged into main
//	nuke   — force-remove ALL worktrees except the main checkout
package main

import (
	"bufio"
	"fmt"
	"os"
	"os/exec"
	"strings"
)

func main() {
	cmd := "list"
	if len(os.Args) > 1 {
		cmd = os.Args[1]
	}

	switch cmd {
	case "list":
		runList()
	case "clean":
		runClean()
	case "nuke":
		runNuke()
	default:
		fmt.Fprintf(os.Stderr, "unknown command: %s\n", cmd)
		os.Exit(1)
	}
}

// worktree holds parsed porcelain output for a single worktree entry.
type worktree struct {
	path   string
	branch string // empty for detached HEAD
}

// parseWorktrees parses `git worktree list --porcelain` output.
func parseWorktrees() ([]worktree, error) {
	out, err := exec.Command("git", "worktree", "list", "--porcelain").Output()
	if err != nil {
		return nil, fmt.Errorf("git worktree list: %w", err)
	}

	var wts []worktree
	var cur worktree
	scanner := bufio.NewScanner(strings.NewReader(string(out)))
	for scanner.Scan() {
		line := scanner.Text()
		switch {
		case strings.HasPrefix(line, "worktree "):
			cur = worktree{path: strings.TrimPrefix(line, "worktree ")}
		case strings.HasPrefix(line, "branch refs/heads/"):
			cur.branch = strings.TrimPrefix(line, "branch refs/heads/")
		case line == "":
			if cur.path != "" {
				wts = append(wts, cur)
			}
			cur = worktree{}
		}
	}
	// last entry if no trailing newline
	if cur.path != "" {
		wts = append(wts, cur)
	}
	return wts, nil
}

// mainWorktree returns the top-level path of the main checkout.
func mainWorktree() (string, error) {
	out, err := exec.Command("git", "rev-parse", "--show-toplevel").Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}

// mergedBranches returns branch names that are fully merged into main.
func mergedBranches() (map[string]bool, error) {
	out, err := exec.Command("git", "branch", "--merged", "main", "--format=%(refname:short)").Output()
	if err != nil {
		return nil, fmt.Errorf("git branch --merged: %w", err)
	}
	m := make(map[string]bool)
	scanner := bufio.NewScanner(strings.NewReader(string(out)))
	for scanner.Scan() {
		b := strings.TrimSpace(scanner.Text())
		if b != "" && b != "main" {
			m[b] = true
		}
	}
	return m, nil
}

func runList() {
	out, err := exec.Command("git", "worktree", "list").CombinedOutput()
	if err != nil {
		fmt.Fprintf(os.Stderr, "git worktree list: %s\n", err)
		os.Exit(1)
	}
	fmt.Print(string(out))
}

func runClean() {
	// Prune stale references first
	if out, err := exec.Command("git", "worktree", "prune").CombinedOutput(); err != nil {
		fmt.Fprintf(os.Stderr, "git worktree prune: %s\n%s", err, out)
		os.Exit(1)
	}

	mainPath, err := mainWorktree()
	if err != nil {
		fmt.Fprintf(os.Stderr, "cannot determine main worktree: %s\n", err)
		os.Exit(1)
	}

	merged, err := mergedBranches()
	if err != nil {
		fmt.Fprintf(os.Stderr, "%s\n", err)
		os.Exit(1)
	}
	if len(merged) == 0 {
		fmt.Println("✅ No merged branches to clean up.")
		return
	}

	wts, err := parseWorktrees()
	if err != nil {
		fmt.Fprintf(os.Stderr, "%s\n", err)
		os.Exit(1)
	}

	// Track which merged branches have associated worktrees
	branchHandled := make(map[string]bool)
	removed := 0

	// Remove worktrees for merged branches
	for _, wt := range wts {
		if wt.path == mainPath || wt.branch == "" {
			continue
		}
		if !merged[wt.branch] {
			continue
		}
		fmt.Printf("🗑️  Removing worktree: %s (branch: %s)\n", wt.path, wt.branch)
		if out, err := exec.Command("git", "worktree", "remove", wt.path).CombinedOutput(); err != nil {
			fmt.Fprintf(os.Stderr, "  ⚠️  failed to remove worktree: %s\n%s", err, out)
			continue
		}
		if out, err := exec.Command("git", "branch", "-d", wt.branch).CombinedOutput(); err != nil {
			fmt.Fprintf(os.Stderr, "  ⚠️  failed to delete branch: %s\n%s", err, out)
		}
		branchHandled[wt.branch] = true
		removed++
	}

	// Delete merged branches that have no worktree
	for branch := range merged {
		if branchHandled[branch] {
			continue
		}
		fmt.Printf("🗑️  Deleting merged branch: %s (no worktree)\n", branch)
		if out, err := exec.Command("git", "branch", "-d", branch).CombinedOutput(); err != nil {
			fmt.Fprintf(os.Stderr, "  ⚠️  failed to delete branch: %s\n%s", err, out)
			continue
		}
		removed++
	}

	fmt.Printf("✅ Cleaned up %d merged worktree(s)/branch(es).\n", removed)
}

func runNuke() {
	mainPath, err := mainWorktree()
	if err != nil {
		fmt.Fprintf(os.Stderr, "cannot determine main worktree: %s\n", err)
		os.Exit(1)
	}

	wts, err := parseWorktrees()
	if err != nil {
		fmt.Fprintf(os.Stderr, "%s\n", err)
		os.Exit(1)
	}

	removed := 0
	for _, wt := range wts {
		if wt.path == mainPath {
			continue
		}
		fmt.Printf("🗑️  Removing: %s\n", wt.path)
		if out, err := exec.Command("git", "worktree", "remove", "--force", wt.path).CombinedOutput(); err != nil {
			fmt.Fprintf(os.Stderr, "  ⚠️  worktree remove failed, cleaning up manually: %s\n%s", err, out)
			os.RemoveAll(wt.path)
		}
		if wt.branch != "" {
			exec.Command("git", "branch", "-D", wt.branch).Run()
		}
		removed++
	}

	exec.Command("git", "worktree", "prune").Run()
	fmt.Printf("✅ Removed %d worktree(s).\n", removed)
}
