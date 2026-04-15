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

	toml "github.com/pelletier/go-toml/v2"
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
	"rara-api":           0, // protobuf-generated type definitions, no workspace deps

	// Layer 1 — core primitives (depend only on layer 0)
	"rara-soul":         1,
	"rara-symphony":     1,
	"rara-skills":       1,
"rara-composio":     1,
	"rara-keyring-store": 1,
	"rara-git":          1,

	// Layer 2 — kernel
	"rara-kernel": 2,

	// Layer 3 — kernel extensions (depend on kernel)
	"rara-codex-oauth":          3,
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

// workspaceProbe is a minimal struct to detect whether a Cargo.toml
// contains a [workspace] section.
type workspaceProbe struct {
	Workspace *struct{} `toml:"workspace"`
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
			var probe workspaceProbe
			if err := toml.Unmarshal(data, &probe); err == nil && probe.Workspace != nil {
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

// cargoWorkspace is used to decode the root Cargo.toml.
type cargoWorkspace struct {
	Workspace struct {
		Dependencies map[string]any `toml:"dependencies"`
	} `toml:"workspace"`
}

// parseWorkspaceAliases extracts workspace crate names from the root
// Cargo.toml by looking for [workspace.dependencies] entries that
// have a `path` field (i.e. local workspace crates, not external deps).
func parseWorkspaceAliases(rootToml string) (map[string]bool, error) {
	data, err := os.ReadFile(rootToml)
	if err != nil {
		return nil, err
	}

	var ws cargoWorkspace
	if err := toml.Unmarshal(data, &ws); err != nil {
		return nil, fmt.Errorf("parsing %s: %w", rootToml, err)
	}

	aliases := make(map[string]bool)
	for name, val := range ws.Workspace.Dependencies {
		// Inline table entries with a path field are workspace crates.
		// e.g. rara-kernel = { path = "crates/kernel" }
		if tbl, ok := val.(map[string]any); ok {
			if _, hasPath := tbl["path"]; hasPath {
				aliases[name] = true
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

// crateCargo is used to decode a crate-level Cargo.toml.
type crateCargo struct {
	Package struct {
		Name string `toml:"name"`
	} `toml:"package"`
	Dependencies      map[string]any `toml:"dependencies"`
	BuildDependencies map[string]any `toml:"build-dependencies"`
	// dev-dependencies are intentionally excluded: they don't affect the
	// runtime dependency graph, so a dev-only import of a higher-layer
	// crate (e.g. a test helper) should not count as a layer violation.
}

// parseCrateDeps extracts the package name and workspace crate dependencies
// from a crate's Cargo.toml file. Only [dependencies] and
// [build-dependencies] are considered; [dev-dependencies] are excluded
// because they do not affect the runtime dependency graph.
func parseCrateDeps(tomlPath string, workspaceCrates map[string]bool) (string, []string, error) {
	data, err := os.ReadFile(tomlPath)
	if err != nil {
		return "", nil, err
	}

	var crate crateCargo
	if err := toml.Unmarshal(data, &crate); err != nil {
		return "", nil, fmt.Errorf("parsing %s: %w", tomlPath, err)
	}

	if crate.Package.Name == "" {
		return "", nil, fmt.Errorf("no package name found in %s", tomlPath)
	}

	var deps []string
	// Collect workspace crate deps from both [dependencies] and [build-dependencies].
	for _, section := range []map[string]any{crate.Dependencies, crate.BuildDependencies} {
		for name, val := range section {
			if !workspaceCrates[name] {
				continue
			}
			// Accept both `dep = { workspace = true }` and `dep.workspace = true`.
			if tbl, ok := val.(map[string]any); ok {
				if ws, exists := tbl["workspace"]; exists {
					if b, ok := ws.(bool); ok && b {
						deps = append(deps, name)
					}
				}
			}
		}
	}

	return crate.Package.Name, deps, nil
}
