// Package deps enforces crate dependency direction rules.
//
// The workspace crates are organized into layers (0 = lowest, 6 = highest).
// A crate at layer N must NOT depend on a crate at layer N+1 or higher.
// Known violations are tracked in an allowlist so existing issues are
// documented while new violations are caught.
package deps

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/urfave/cli/v3"
)

// Cmd returns the top-level "check-deps" command.
func Cmd() *cli.Command {
	return &cli.Command{
		Name:  "check-deps",
		Usage: "Check crate dependency direction rules",
		Action: func(_ context.Context, _ *cli.Command) error {
			return runCheckDeps()
		},
	}
}

// layerMap assigns each workspace crate (by Cargo package name) to a layer.
//
// Layer 0 — foundation: no workspace dependencies
// Layer 1 — core primitives: depend only on layer 0
// Layer 2 — kernel: the central orchestration crate
// Layer 3 — kernel extensions: depend on kernel
// Layer 4 — integration: depend on layer 3 crates
// Layer 5 — application: wire everything together
// Layer 6 — entry: binary crates and API
var layerMap = map[string]int{
	// Layer 0 — foundation
	"base":               0,
	"rara-error":         0,
	"common-runtime":     0,
	"common-telemetry":   0,
	"common-worker":      0,
	"yunara-store":       0,
	"rara-tool-macro":    0,
	"crawl4ai":           0,
	"rara-paths":         0,
	"rara-model":         0,
	"rara-domain-shared": 0,

	// Layer 1 — core primitives (depend only on layer 0)
	"rara-soul":         1,
	"rara-symphony":     1,
	"rara-skills":       1,
	"rara-vault":        1,
	"rara-composio":     1,
	"rara-keyring-store": 1,
	"rara-codex-oauth":  1,
	"rara-git":          1,

	// Layer 2 — kernel
	"rara-kernel": 2,

	// Layer 3 — kernel extensions (depend on kernel)
	"rara-dock":                 3,
	"rara-sessions":             3,
	"rara-agents":               3,
	"rara-mcp":                  3,
	"rara-pg-credential-store":  3,

	// Layer 4 — integration (depend on layer 3 crates)
	"rara-channels":      4,
	"rara-backend-admin": 4,

	// Layer 5 — application
	"rara-app":    5,
	"rara-server": 5,

	// Layer 6 — entry
	"rara-cli": 6,
	"rara-api": 0, // protobuf-generated type definitions, no workspace deps
}

// allowedViolations lists known dependency direction violations that
// existed before this check was introduced. Each entry is "from -> to".
// Remove entries as they are fixed.
var allowedViolations = map[string]bool{
	// kernel (layer 2) depends on rara-soul (layer 1) — this is actually fine,
	// higher layers can depend on lower layers. Only reverse is a violation.
}

// violation records a single dependency direction breach.
type violation struct {
	From      string
	FromLayer int
	To        string
	ToLayer   int
}

func (v violation) String() string {
	return fmt.Sprintf("%s (layer %d) -> %s (layer %d)", v.From, v.FromLayer, v.To, v.ToLayer)
}

func (v violation) key() string {
	return fmt.Sprintf("%s -> %s", v.From, v.To)
}

func runCheckDeps() error {
	// Find the workspace root by looking for the root Cargo.toml
	root, err := findWorkspaceRoot()
	if err != nil {
		return fmt.Errorf("finding workspace root: %w", err)
	}

	fmt.Printf("Workspace root: %s\n", root)

	// Parse workspace dependency aliases from root Cargo.toml
	aliases, err := parseWorkspaceAliases(filepath.Join(root, "Cargo.toml"))
	if err != nil {
		return fmt.Errorf("parsing workspace aliases: %w", err)
	}

	// Find all crate Cargo.toml files
	crateTomlFiles, err := findCrateTomlFiles(root)
	if err != nil {
		return fmt.Errorf("finding crate Cargo.toml files: %w", err)
	}

	var violations []violation
	var unknownCrates []string

	for _, tomlPath := range crateTomlFiles {
		pkgName, deps, err := parseCrateDeps(tomlPath, aliases)
		if err != nil {
			fmt.Fprintf(os.Stderr, "warning: skipping %s: %v\n", tomlPath, err)
			continue
		}

		fromLayer, known := layerMap[pkgName]
		if !known {
			unknownCrates = append(unknownCrates, pkgName)
			continue
		}

		for _, dep := range deps {
			toLayer, depKnown := layerMap[dep]
			if !depKnown {
				// External or unknown dependency — skip
				continue
			}
			if toLayer > fromLayer {
				v := violation{
					From:      pkgName,
					FromLayer: fromLayer,
					To:        dep,
					ToLayer:   toLayer,
				}
				if !allowedViolations[v.key()] {
					violations = append(violations, v)
				}
			}
		}
	}

	// Report unknown crates
	if len(unknownCrates) > 0 {
		sort.Strings(unknownCrates)
		fmt.Println("\nWarning: crates not in layer map (add them to scripts/internal/deps/commands.go):")
		for _, c := range unknownCrates {
			fmt.Printf("  - %s\n", c)
		}
	}

	// Report violations
	if len(violations) > 0 {
		sort.Slice(violations, func(i, j int) bool {
			return violations[i].String() < violations[j].String()
		})
		fmt.Println("\nDependency direction violations found:")
		for _, v := range violations {
			fmt.Printf("  ERROR: %s\n", v)
		}
		fmt.Printf("\n%d violation(s) found. A lower-layer crate must not depend on a higher-layer crate.\n", len(violations))
		fmt.Println("If this is intentional, add it to allowedViolations in scripts/internal/deps/commands.go")
		return fmt.Errorf("dependency check failed with %d violation(s)", len(violations))
	}

	fmt.Println("\nAll dependency direction checks passed.")
	return nil
}

// findWorkspaceRoot walks up from cwd to find the directory containing
// a Cargo.toml with [workspace].
func findWorkspaceRoot() (string, error) {
	dir, err := os.Getwd()
	if err != nil {
		return "", err
	}
	for {
		candidate := filepath.Join(dir, "Cargo.toml")
		if data, err := os.ReadFile(candidate); err == nil {
			if strings.Contains(string(data), "[workspace]") {
				return dir, nil
			}
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", fmt.Errorf("no workspace Cargo.toml found")
		}
		dir = parent
	}
}

// parseWorkspaceAliases extracts the alias-to-package mapping from
// [workspace.dependencies] entries that have a path = "..." field.
// For example: `rara-kernel = { path = "crates/kernel" }` maps
// the alias "rara-kernel" to whatever package name is declared
// in that path's Cargo.toml.
//
// We use a simpler approach: the alias IS the package name as used
// in dependency declarations. We just need to know which aliases
// are workspace crates (have a path).
func parseWorkspaceAliases(rootToml string) (map[string]bool, error) {
	data, err := os.ReadFile(rootToml)
	if err != nil {
		return nil, err
	}

	aliases := make(map[string]bool)
	lines := strings.Split(string(data), "\n")
	inWorkspaceDeps := false

	for _, line := range lines {
		trimmed := strings.TrimSpace(line)

		if trimmed == "[workspace.dependencies]" {
			inWorkspaceDeps = true
			continue
		}
		// New section starts
		if strings.HasPrefix(trimmed, "[") && trimmed != "[workspace.dependencies]" {
			if inWorkspaceDeps {
				inWorkspaceDeps = false
			}
			continue
		}

		if !inWorkspaceDeps {
			continue
		}

		// Look for lines like: rara-kernel = { path = "crates/kernel" }
		if strings.Contains(trimmed, "path =") {
			parts := strings.SplitN(trimmed, "=", 2)
			if len(parts) >= 1 {
				alias := strings.TrimSpace(parts[0])
				aliases[alias] = true
			}
		}
	}

	return aliases, nil
}

// findCrateTomlFiles finds all Cargo.toml files in the crates/ directory
// and the api/ directory.
func findCrateTomlFiles(root string) ([]string, error) {
	var files []string

	// Walk crates/ directory
	cratesDir := filepath.Join(root, "crates")
	err := filepath.Walk(cratesDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if info.Name() == "Cargo.toml" && !info.IsDir() {
			files = append(files, path)
		}
		return nil
	})
	if err != nil {
		return nil, err
	}

	// Also check api/
	apiToml := filepath.Join(root, "api", "Cargo.toml")
	if _, err := os.Stat(apiToml); err == nil {
		files = append(files, apiToml)
	}

	return files, nil
}

// parseCrateDeps extracts the package name and workspace crate dependencies
// from a crate's Cargo.toml file.
func parseCrateDeps(tomlPath string, workspaceCrates map[string]bool) (string, []string, error) {
	data, err := os.ReadFile(tomlPath)
	if err != nil {
		return "", nil, err
	}

	lines := strings.Split(string(data), "\n")

	// Extract package name
	pkgName := ""
	inPackage := false
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "[package]" {
			inPackage = true
			continue
		}
		if strings.HasPrefix(trimmed, "[") && trimmed != "[package]" {
			inPackage = false
			continue
		}
		if inPackage && strings.HasPrefix(trimmed, "name") {
			parts := strings.SplitN(trimmed, "=", 2)
			if len(parts) == 2 {
				pkgName = strings.Trim(strings.TrimSpace(parts[1]), "\"")
			}
		}
	}

	if pkgName == "" {
		return "", nil, fmt.Errorf("no package name found in %s", tomlPath)
	}

	// Extract dependencies that are workspace crates
	var deps []string
	inDeps := false
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)

		// Match [dependencies], [dev-dependencies], [dependencies.*]
		if strings.HasPrefix(trimmed, "[") {
			inDeps = trimmed == "[dependencies]" ||
				strings.HasPrefix(trimmed, "[dependencies.")
			continue
		}

		if !inDeps {
			continue
		}

		// Skip empty lines and comments
		if trimmed == "" || strings.HasPrefix(trimmed, "#") {
			continue
		}

		// Extract dependency name
		parts := strings.SplitN(trimmed, "=", 2)
		if len(parts) < 2 {
			continue
		}
		depName := strings.TrimSpace(parts[0])
		depValue := strings.TrimSpace(parts[1])

		// Only care about workspace dependencies
		if !strings.Contains(depValue, "workspace") {
			continue
		}

		// Check if this is a known workspace crate
		if workspaceCrates[depName] {
			deps = append(deps, depName)
		}
	}

	return pkgName, deps, nil
}
