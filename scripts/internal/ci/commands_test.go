package ci

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	toml "github.com/pelletier/go-toml/v2"
)

const (
	repoRoot         = "../../.."
	linuxArm64Runner = "ubuntu-24.04-arm"
	linuxArm64Target = "aarch64-unknown-linux-gnu"
)

func TestCargoDistBuildsLinuxArm64OnArmRunner(t *testing.T) {
	data := readRepoFile(t, "Cargo.toml")

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
		t.Fatalf("parse Cargo.toml: %v", err)
	}

	if !contains(cfg.Workspace.Metadata.Dist.Targets, linuxArm64Target) {
		t.Fatalf("cargo-dist targets = %v, want %s", cfg.Workspace.Metadata.Dist.Targets, linuxArm64Target)
	}
	if got := cfg.Workspace.Metadata.Dist.GithubCustomRunners[linuxArm64Target]; got != linuxArm64Runner {
		t.Fatalf("custom runner for %s = %q, want %q", linuxArm64Target, got, linuxArm64Runner)
	}
}

func TestRustWorkflowRunsOnX64AndArm64Linux(t *testing.T) {
	data := readRepoFile(t, ".github/workflows/rust.yml")

	text := string(data)
	if !strings.Contains(text, linuxArm64Runner) {
		t.Fatalf("rust.yml does not mention runner %q", linuxArm64Runner)
	}
	if !strings.Contains(text, "${{ matrix.runner }}") {
		t.Fatalf("rust.yml should use matrix.runner for Rust jobs")
	}
}

func TestArm64LinuxTargetKeepsWarningsAsErrors(t *testing.T) {
	data := readRepoFile(t, ".cargo/config.toml")

	var cfg map[string]any
	if err := toml.Unmarshal(data, &cfg); err != nil {
		t.Fatalf("parse .cargo/config.toml: %v", err)
	}

	targets, ok := cfg["target"].(map[string]any)
	if !ok {
		t.Fatalf(".cargo/config.toml has no [target] table")
	}
	target, ok := targets[linuxArm64Target].(map[string]any)
	if !ok {
		t.Fatalf(".cargo/config.toml has no [target.%s] table", linuxArm64Target)
	}
	flags, ok := target["rustflags"].([]any)
	if !ok {
		t.Fatalf("[target.%s] has no rustflags array", linuxArm64Target)
	}
	if !containsAny(flags, "-D") || !containsAny(flags, "warnings") {
		t.Fatalf("[target.%s] rustflags = %v, want -D warnings", linuxArm64Target, flags)
	}
}

func readRepoFile(t *testing.T, name string) []byte {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(repoRoot, name))
	if err != nil {
		t.Fatalf("read %s: %v", name, err)
	}
	return data
}

func contains(items []string, needle string) bool {
	for _, item := range items {
		if item == needle {
			return true
		}
	}
	return false
}

func containsAny(items []any, needle string) bool {
	for _, item := range items {
		if s, ok := item.(string); ok && s == needle {
			return true
		}
	}
	return false
}

func TestGateWorkflowsDoNotTargetRetiredRunner(t *testing.T) {
	for _, wf := range gateWorkflows {
		data := readRepoFile(t, wf)
		for _, line := range strings.Split(string(data), "\n") {
			trimmed := strings.TrimSpace(line)
			if trimmed == "" || strings.HasPrefix(trimmed, "#") {
				continue // comments may explain why the runner is retired
			}
			if strings.Contains(trimmed, retiredRunner) {
				t.Errorf("%s references retired runner %q — gate jobs would queue forever (#2166)", wf, retiredRunner)
			}
		}
	}
}
