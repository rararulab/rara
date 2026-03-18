package quality

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
)

// CrateInfo holds quality metrics for a single crate.
type CrateInfo struct {
	Name       string
	Dir        string // relative path from repo root
	Layer      string
	HasAgentMD bool
	HasTests   bool
	PubItems   int
	DocItems   int
	LOC        int
}

// DocPercent returns the documentation coverage as a percentage.
func (c *CrateInfo) DocPercent() int {
	if c.PubItems == 0 {
		return 0
	}
	return (c.DocItems * 100) / c.PubItems
}

// findRepoRoot walks up from cwd looking for a Cargo.toml workspace root.
func findRepoRoot() (string, error) {
	dir, err := os.Getwd()
	if err != nil {
		return "", err
	}
	for {
		if _, err := os.Stat(filepath.Join(dir, "Cargo.toml")); err == nil {
			if _, err := os.Stat(filepath.Join(dir, "crates")); err == nil {
				return dir, nil
			}
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", fmt.Errorf("could not find repo root (no Cargo.toml with crates/ directory)")
		}
		dir = parent
	}
}

// discoverCrates finds all crates under the crates/ directory.
func discoverCrates(root string) ([]CrateInfo, error) {
	cratesDir := filepath.Join(root, "crates")
	var crates []CrateInfo

	err := filepath.Walk(cratesDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return nil // skip inaccessible paths
		}
		if info.Name() != "Cargo.toml" {
			return nil
		}

		crateDir := filepath.Dir(path)
		relDir, _ := filepath.Rel(root, crateDir)

		name, err := parseCrateName(path)
		if err != nil {
			return nil // skip unparseable Cargo.toml
		}

		ci := CrateInfo{
			Name:  name,
			Dir:   relDir,
			Layer: determineLayer(relDir),
		}

		ci.HasAgentMD = fileExists(filepath.Join(crateDir, "AGENT.md"))
		ci.HasTests = detectTests(crateDir)
		ci.PubItems, ci.DocItems = countDocCoverage(crateDir)
		ci.LOC = countLOC(crateDir)

		crates = append(crates, ci)
		return nil
	})
	if err != nil {
		return nil, err
	}

	// Sort by LOC descending for consistent output
	sort.Slice(crates, func(i, j int) bool {
		return crates[i].LOC > crates[j].LOC
	})

	return crates, nil
}

// parseCrateName extracts the `name = "..."` field from Cargo.toml.
func parseCrateName(cargoPath string) (string, error) {
	f, err := os.Open(cargoPath)
	if err != nil {
		return "", err
	}
	defer f.Close()

	scanner := bufio.NewScanner(f)
	re := regexp.MustCompile(`^name\s*=\s*"([^"]+)"`)
	for scanner.Scan() {
		if m := re.FindStringSubmatch(scanner.Text()); m != nil {
			return m[1], nil
		}
	}
	return "", fmt.Errorf("no name field in %s", cargoPath)
}

// determineLayer classifies a crate based on its directory path.
func determineLayer(relDir string) string {
	parts := strings.Split(relDir, string(os.PathSeparator))
	if len(parts) < 2 {
		return "unknown"
	}
	// parts[0] = "crates", parts[1] = category or crate name
	switch parts[1] {
	case "common":
		return "common"
	case "domain":
		return "domain"
	case "extensions":
		return "extensions"
	case "integrations":
		return "integrations"
	case "kernel":
		return "kernel"
	case "server":
		return "server"
	case "cmd":
		return "cmd"
	default:
		// Top-level crates under crates/ that aren't in a category
		// e.g. crates/app, crates/skills, crates/soul, etc.
		return "app"
	}
}

// fileExists checks if a path exists and is a regular file.
func fileExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}

// detectTests checks for #[cfg(test)] in .rs files or a tests/ directory.
func detectTests(crateDir string) bool {
	// Check for tests/ directory
	if info, err := os.Stat(filepath.Join(crateDir, "tests")); err == nil && info.IsDir() {
		return true
	}

	found := false
	re := regexp.MustCompile(`#\[cfg\(test\)\]`)

	_ = filepath.Walk(crateDir, func(path string, info os.FileInfo, err error) error {
		if err != nil || found {
			return nil
		}
		if !strings.HasSuffix(path, ".rs") {
			return nil
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return nil
		}
		if re.Match(data) {
			found = true
		}
		return nil
	})

	return found
}

var (
	// Matches pub fn, pub struct, pub enum, pub trait declarations
	pubItemRe = regexp.MustCompile(`^\s*pub\s+(?:fn|struct|enum|trait)\s+\w+`)
	// Matches doc comment lines
	docCommentRe = regexp.MustCompile(`^\s*///`)
)

// countDocCoverage counts public items and how many have doc comments.
func countDocCoverage(crateDir string) (pubItems, docItems int) {
	_ = filepath.Walk(crateDir, func(path string, info os.FileInfo, err error) error {
		if err != nil || !strings.HasSuffix(path, ".rs") {
			return nil
		}

		f, err := os.Open(path)
		if err != nil {
			return nil
		}
		defer f.Close()

		scanner := bufio.NewScanner(f)
		var lines []string
		for scanner.Scan() {
			lines = append(lines, scanner.Text())
		}

		for i, line := range lines {
			if pubItemRe.MatchString(line) {
				pubItems++
				// Look backwards for doc comments immediately before this item
				if hasDocComment(lines, i) {
					docItems++
				}
			}
		}
		return nil
	})
	return
}

// hasDocComment checks if lines immediately before index i contain a /// comment.
func hasDocComment(lines []string, idx int) bool {
	for j := idx - 1; j >= 0; j-- {
		trimmed := strings.TrimSpace(lines[j])
		if docCommentRe.MatchString(lines[j]) {
			return true
		}
		// Skip attribute lines like #[derive(...)], #[serde(...)], etc.
		if strings.HasPrefix(trimmed, "#[") || strings.HasPrefix(trimmed, "//") || trimmed == "" {
			continue
		}
		break
	}
	return false
}

// countLOC counts total lines in all .rs files in the crate directory.
func countLOC(crateDir string) int {
	total := 0
	_ = filepath.Walk(crateDir, func(path string, info os.FileInfo, err error) error {
		if err != nil || !strings.HasSuffix(path, ".rs") {
			return nil
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return nil
		}
		total += strings.Count(string(data), "\n")
		if len(data) > 0 && data[len(data)-1] != '\n' {
			total++ // count last line if no trailing newline
		}
		return nil
	})
	return total
}
