// git.go provides low-level git worktree operations.
package worktree

import (
	"bufio"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
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
	Path       string
	Branch     string // empty for detached HEAD
	IsMain     bool
	Prunable   bool
	Locked     bool      // worktree has a lock file
	IsCurrent  bool      // worktree is the current working directory
	Status     Status
	LastActive time.Time // last modification time of the worktree directory
	DiskSize   int64     // total disk usage in bytes
}

// Protected returns true if the worktree cannot be deleted.
func (e Entry) Protected() bool {
	return e.IsMain || e.Locked || e.IsCurrent
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

	// Detect current working directory to mark the active worktree
	cwd, _ := os.Getwd()

	var entries []Entry
	var cur Entry
	prunable := false
	locked := false

	// finalizeEntry fills computed fields and returns the entry ready for collection.
	finalizeEntry := func(e Entry) Entry {
		e.IsMain = e.Path == mainPath
		e.Prunable = prunable
		e.Locked = locked
		e.IsCurrent = isSameOrChild(cwd, e.Path)
		e.Status = classifyEntry(e, merged)
		// Populate LastActive for non-prunable entries with existing paths
		if !e.Prunable {
			if _, err := os.Stat(e.Path); err == nil {
				e.LastActive = lastActiveTime(e.Path)
			}
		}
		return e
	}

	scanner := bufio.NewScanner(strings.NewReader(string(out)))
	for scanner.Scan() {
		line := scanner.Text()
		switch {
		case strings.HasPrefix(line, "worktree "):
			cur = Entry{Path: strings.TrimPrefix(line, "worktree ")}
			prunable = false
			locked = false
		case strings.HasPrefix(line, "branch refs/heads/"):
			cur.Branch = strings.TrimPrefix(line, "branch refs/heads/")
		case line == "prunable":
			prunable = true
		case line == "locked", strings.HasPrefix(line, "locked "):
			locked = true
		case line == "":
			if cur.Path != "" {
				entries = append(entries, finalizeEntry(cur))
			}
			cur = Entry{}
		}
	}
	if cur.Path != "" {
		entries = append(entries, finalizeEntry(cur))
	}
	return entries, nil
}

// dirSize computes total disk usage of a directory tree in bytes.
// Returns 0 on any error.
func dirSize(path string) int64 {
	var total int64
	_ = filepath.WalkDir(path, func(_ string, d fs.DirEntry, err error) error {
		if err != nil {
			return nil // skip unreadable entries
		}
		if !d.IsDir() {
			if info, err := d.Info(); err == nil {
				total += info.Size()
			}
		}
		return nil
	})
	return total
}

// lastActiveTime returns the most recent modification time among key git files
// in the worktree (.git, HEAD, index), providing a meaningful "last active" signal.
// Falls back to the directory mtime if no git files are found.
func lastActiveTime(path string) time.Time {
	var latest time.Time
	// Check git-related files that change on commits and checkouts
	candidates := []string{
		filepath.Join(path, ".git"),
		filepath.Join(path, "HEAD"),
		filepath.Join(path, ".git", "HEAD"),
		filepath.Join(path, ".git", "index"),
	}
	for _, c := range candidates {
		if info, err := os.Stat(c); err == nil {
			if info.ModTime().After(latest) {
				latest = info.ModTime()
			}
		}
	}
	// Fall back to directory mtime
	if latest.IsZero() {
		if info, err := os.Stat(path); err == nil {
			latest = info.ModTime()
		}
	}
	return latest
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

// isSameOrChild returns true if child is the same as or under parent directory.
func isSameOrChild(child, parent string) bool {
	c, err1 := filepath.EvalSymlinks(child)
	p, err2 := filepath.EvalSymlinks(parent)
	if err1 != nil || err2 != nil {
		return child == parent
	}
	return c == p || strings.HasPrefix(c, p+string(os.PathSeparator))
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
