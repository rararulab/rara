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

	// The self-hosted ARC fleet is gone (#2166); any job targeting it
	// queues for 24h and is cancelled by GitHub. It must never appear in
	// a workflow that a required merge-gate check depends on.
	retiredRunner = "arc-runner-set"
)

// gateWorkflows are the workflows whose jobs feed the required
// branch-protection checks (Rust Success / Lint Success).
var gateWorkflows = []string{
	".github/workflows/ci.yml",
	".github/workflows/rust.yml",
	".github/workflows/lint.yml",
}

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

	if err := checkCargoDist(filepath.Join(root, "Cargo.toml")); err != nil {
		return err
	}
	// The merge-gate Rust workflow runs x64-only since #2228; the arm64 test
	// leg was removed for CI latency. arm64 coverage now comes from the
	// release build (checkCargoDist above) + the .cargo/config.toml arm64
	// stanza below, so rust.yml is no longer required to mention the arm64
	// runner.
	if err := checkContains(filepath.Join(root, ".cargo/config.toml"), []byte("[target."+arm64Target+"]")); err != nil {
		return err
	}
	for _, wf := range gateWorkflows {
		if err := checkNotContains(filepath.Join(root, wf), []byte(retiredRunner)); err != nil {
			return err
		}
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

// checkNotContains fails when any non-comment line of the file references
// the needle. YAML comment lines are skipped so workflows can still explain
// WHY the retired runner must not come back.
func checkNotContains(path string, needle []byte) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	for _, line := range bytes.Split(data, []byte("\n")) {
		trimmed := bytes.TrimSpace(line)
		if len(trimmed) == 0 || trimmed[0] == '#' {
			continue
		}
		if bytes.Contains(trimmed, needle) {
			return fmt.Errorf("%s must not reference %q (retired runner fleet, see #2166)", path, needle)
		}
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
