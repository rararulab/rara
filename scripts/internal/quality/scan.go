package quality

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"

	toml "github.com/pelletier/go-toml/v2"
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
		ci.LOC = countTotalLines(crateDir)

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

// parseCrateName extracts the [package].name field from Cargo.toml.
func parseCrateName(cargoPath string) (string, error) {
	data, err := os.ReadFile(cargoPath)
	if err != nil {
		return "", err
	}

	var cargo struct {
		Package struct {
			Name string `toml:"name"`
		} `toml:"package"`
	}
	if err := toml.Unmarshal(data, &cargo); err != nil {
		return "", fmt.Errorf("parsing %s: %w", cargoPath, err)
	}
	if cargo.Package.Name == "" {
		return "", fmt.Errorf("no name field in %s", cargoPath)
	}
	return cargo.Package.Name, nil
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
// When searching upward, only doc comments (///), inner doc comments (//!),
// attributes (#[...]), and blank lines are skipped. A regular // comment
// terminates the search — it is not a doc comment and should not be
// "transparent" to a /// further above.
func hasDocComment(lines []string, idx int) bool {
	for j := idx - 1; j >= 0; j-- {
		trimmed := strings.TrimSpace(lines[j])
		if docCommentRe.MatchString(lines[j]) {
			return true
		}
		// Skip blank lines and attributes.
		if trimmed == "" || strings.HasPrefix(trimmed, "#[") {
			continue
		}
		// Skip inner doc comments (//!).
		if strings.HasPrefix(trimmed, "//!") {
			continue
		}
		// A regular // comment is NOT a doc comment — stop searching.
		// This prevents false positives where a /// far above a //
		// comment would incorrectly count as documentation.
		break
	}
	return false
}

// countTotalLines counts total lines (including blanks and comments) in all
// .rs files in the crate directory. This is not traditional LOC which
// excludes blanks/comments — it is a raw line count used for crate sizing.
func countTotalLines(crateDir string) int {
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
