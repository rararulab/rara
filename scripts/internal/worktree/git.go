// git.go provides low-level git worktree operations.
package worktree

import (
	"bufio"
	"fmt"
	"os/exec"
	"strings"
)

// Status describes the state of a worktree entry.
type Status int

const (
	StatusActive   Status = iota // branch exists, not merged
	StatusMerged                 // branch fully merged into main
	StatusDetached               // detached HEAD (no branch)
	StatusPrunable               // stale reference, can be pruned
)

// String returns a human-readable label for the status.
func (s Status) String() string {
	switch s {
	case StatusMerged:
		return "merged"
	case StatusDetached:
		return "detached"
	case StatusPrunable:
		return "prunable"
	default:
		return "active"
	}
}

// Entry holds parsed porcelain output for a single git worktree.
type Entry struct {
	Path     string
	Branch   string // empty for detached HEAD
	IsMain   bool
	Prunable bool
	Status   Status
}

// List parses `git worktree list --porcelain` and returns all entries,
// enriched with merge status information.
func List() ([]Entry, error) {
	out, err := exec.Command("git", "worktree", "list", "--porcelain").Output()
	if err != nil {
		return nil, fmt.Errorf("git worktree list: %w", err)
	}

	mainPath, err := MainPath()
	if err != nil {
		return nil, err
	}

	merged, err := MergedBranches()
	if err != nil {
		return nil, err
	}

	var entries []Entry
	var cur Entry
	prunable := false
	scanner := bufio.NewScanner(strings.NewReader(string(out)))
	for scanner.Scan() {
		line := scanner.Text()
		switch {
		case strings.HasPrefix(line, "worktree "):
			cur = Entry{Path: strings.TrimPrefix(line, "worktree ")}
			prunable = false
		case strings.HasPrefix(line, "branch refs/heads/"):
			cur.Branch = strings.TrimPrefix(line, "branch refs/heads/")
		case line == "prunable":
			prunable = true
		case line == "":
			cur.IsMain = cur.Path == mainPath
			cur.Prunable = prunable
			cur.Status = classifyEntry(cur, merged)
			if cur.Path != "" {
				entries = append(entries, cur)
			}
			cur = Entry{}
		}
	}
	if cur.Path != "" {
		cur.IsMain = cur.Path == mainPath
		cur.Prunable = prunable
		cur.Status = classifyEntry(cur, merged)
		entries = append(entries, cur)
	}
	return entries, nil
}

func classifyEntry(e Entry, merged map[string]bool) Status {
	if e.Prunable {
		return StatusPrunable
	}
	if e.Branch == "" {
		return StatusDetached
	}
	if merged[e.Branch] {
		return StatusMerged
	}
	return StatusActive
}

// MainPath returns the top-level path of the main checkout.
func MainPath() (string, error) {
	out, err := exec.Command("git", "rev-parse", "--show-toplevel").Output()
	if err != nil {
		return "", fmt.Errorf("git rev-parse --show-toplevel: %w", err)
	}
	return strings.TrimSpace(string(out)), nil
}

// MergedBranches returns branch names that are fully merged into main.
func MergedBranches() (map[string]bool, error) {
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

// Prune runs `git worktree prune` to clean stale references.
func Prune() error {
	if out, err := exec.Command("git", "worktree", "prune").CombinedOutput(); err != nil {
		return fmt.Errorf("git worktree prune: %w\n%s", err, out)
	}
	return nil
}

// Remove removes a worktree at the given path.
func Remove(path string, force bool) error {
	args := []string{"worktree", "remove"}
	if force {
		args = append(args, "--force")
	}
	args = append(args, path)
	if out, err := exec.Command("git", args...).CombinedOutput(); err != nil {
		return fmt.Errorf("git worktree remove %s: %w\n%s", path, err, out)
	}
	return nil
}

// DeleteBranch deletes a local branch. If force is true, uses -D instead of -d.
func DeleteBranch(name string, force bool) error {
	flag := "-d"
	if force {
		flag = "-D"
	}
	if out, err := exec.Command("git", "branch", flag, name).CombinedOutput(); err != nil {
		return fmt.Errorf("git branch %s %s: %w\n%s", flag, name, err, out)
	}
	return nil
}
