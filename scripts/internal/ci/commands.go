// Package ci enforces repository CI runner invariants.
package ci

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"path/filepath"

	toml "github.com/pelletier/go-toml/v2"
	"github.com/urfave/cli/v3"
)

const (
	arm64Runner = "ubuntu-24.04-arm"
	arm64Target = "aarch64-unknown-linux-gnu"

	// The self-hosted ARC fleet came back online 2026-07-11 and is the
	// Rust merge-gate runner again (#2226). #2166 had moved the gate to
	// GitHub-hosted runners after arc went away ~2026-05-08, but the
	// GitHub-hosted arm64 pool then hung the gate systematically. Lock
	// rust.yml to reference this runner so a silent switch back to a hung
	// GitHub-hosted runner is caught. NOTE: this reverses #2166's earlier
	// "arc-runner-set is retired, never reference it" invariant on the
	// deliberate premise correction that the fleet is no longer retired.
	rustGateRunner = "arc-runner-set"
)

// Cmd returns the "check-ci-runners" command.
func Cmd() *cli.Command {
	return &cli.Command{
		Name:  "check-ci-runners",
		Usage: "Check CI runner coverage for Linux arm64",
		Action: func(_ context.Context, _ *cli.Command) error {
			return runCheck()
		},
	}
}

func runCheck() error {
	root, err := findRepoRoot()
	if err != nil {
		return err
	}

	// arm64 *release-artifact* invariants — unaffected by the CI merge-gate
	// runner switch. cargo-dist still cross-builds the aarch64-linux binary
	// (release.yml, push/tag only) and .cargo/config.toml carries its
	// warnings-as-errors stanza.
	if err := checkCargoDist(filepath.Join(root, "Cargo.toml")); err != nil {
		return err
	}
	if err := checkContains(filepath.Join(root, ".cargo/config.toml"), []byte("[target."+arm64Target+"]")); err != nil {
		return err
	}
	// Merge-gate runner invariant: the Rust gate must run on the restored
	// self-hosted arc-runner-set fleet (#2226), not a GitHub-hosted runner
	// whose arm64 pool hung the gate.
	if err := checkContains(filepath.Join(root, ".github/workflows/rust.yml"), []byte(rustGateRunner)); err != nil {
		return err
	}

	fmt.Println("CI runner checks passed.")
	return nil
}

func findRepoRoot() (string, error) {
	dir, err := os.Getwd()
	if err != nil {
		return "", err
	}
	for {
		if _, err := os.Stat(filepath.Join(dir, ".git")); err == nil {
			return dir, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", fmt.Errorf("no git repository root found")
		}
		dir = parent
	}
}

func checkCargoDist(path string) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}

	var cfg struct {
		Workspace struct {
			Metadata struct {
				Dist struct {
					Targets             []string          `toml:"targets"`
					GithubCustomRunners map[string]string `toml:"github-custom-runners"`
				} `toml:"dist"`
			} `toml:"metadata"`
		} `toml:"workspace"`
	}
	if err := toml.Unmarshal(data, &cfg); err != nil {
		return fmt.Errorf("parse %s: %w", path, err)
	}

	if !containsString(cfg.Workspace.Metadata.Dist.Targets, arm64Target) {
		return fmt.Errorf("cargo-dist targets must include %s", arm64Target)
	}
	if got := cfg.Workspace.Metadata.Dist.GithubCustomRunners[arm64Target]; got != arm64Runner {
		return fmt.Errorf("cargo-dist runner for %s is %q, want %q", arm64Target, got, arm64Runner)
	}
	return nil
}

func checkContains(path string, needle []byte) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	if !bytes.Contains(data, needle) {
		return fmt.Errorf("%s must contain %q", path, needle)
	}
	return nil
}

func containsString(items []string, needle string) bool {
	for _, item := range items {
		if item == needle {
			return true
		}
	}
	return false
}
