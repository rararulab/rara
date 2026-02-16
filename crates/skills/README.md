# rara-skills

Skill discovery, management, and prompt-injection library for the rara platform.

Skills are defined as `SKILL.md` files with YAML frontmatter and a Markdown body containing instructions for the LLM. The library supports multiple discovery sources (project-local, personal, plugins, and registry-installed) and generates XML blocks for system prompt injection.

## Architecture

```
src/
  lib.rs            Crate root, module declarations
  types.rs          Core types: SkillMetadata, SkillContent, SkillsManifest, RepoEntry, etc.
  error.rs          SkillError enum (snafu-based)
  discover.rs       SkillDiscoverer trait + FsSkillDiscoverer implementation
  parse.rs          SKILL.md parser: frontmatter extraction, YAML deserialization, name validation
  registry.rs       SkillRegistry async trait + InMemoryRegistry
  formats.rs        Plugin format detection (PluginFormat enum) + ClaudeCodeAdapter
  install.rs        Git repo installation via HTTP tarball download + extraction
  manifest.rs       ManifestStore for persistent JSON manifest with atomic writes
  requirements.rs   Binary requirement checking (check_bin, check_requirements) and install execution
  prompt_gen.rs     generate_skills_prompt() for LLM system prompt injection
  watcher.rs        SkillWatcher using notify-debouncer-full for filesystem change events
  loader.rs         Legacy single-file skill parser (internal only, will be removed)
```

### Module Overview

- **`types.rs`** -- Core types: `SkillMetadata`, `SkillContent`, `SkillsManifest`, `RepoEntry`, `SkillRequirements`, `InstallSpec`, `SkillEligibility`. Also legacy `Skill`/`SkillMetadataLegacy` for backward compatibility.
- **`error.rs`** -- `SkillError` enum using snafu (`Io`, `Frontmatter`, `MissingFrontmatter`, `InvalidInput`, `NotFound`, `NotAllowed`, `Request`, `Archive`, `Watcher`, etc.).
- **`discover.rs`** -- `SkillDiscoverer` trait + `FsSkillDiscoverer` implementation. Discovers skills from project/personal/plugin/registry paths.
- **`parse.rs`** -- SKILL.md parser. Splits frontmatter from body, deserializes YAML metadata, validates skill names, merges OpenClaw-style requirements.
- **`registry.rs`** -- `SkillRegistry` async trait + `InMemoryRegistry` concrete implementation. Sync convenience methods (`list_all`, `get`, `remove`) for use in `std::sync::RwLock` contexts.
- **`formats.rs`** -- Plugin format detection (`PluginFormat` enum: `Skill`/`ClaudeCode`/`Codex`/`Generic`) + `ClaudeCodeAdapter` for scanning `.claude-plugin/` repos.
- **`install.rs`** -- Git repo installation via HTTP tarball download + extraction. Uses `ManifestStore` for tracking installed repos.
- **`manifest.rs`** -- `ManifestStore` for persistent JSON manifest with atomic writes (write to `.tmp`, then rename).
- **`requirements.rs`** -- Binary requirement checking (`check_bin`, `check_requirements`) and install command execution (`run_install`, `install_command_preview`).
- **`prompt_gen.rs`** -- `generate_skills_prompt()` generates an `<available_skills>` XML block for LLM system prompt injection.
- **`watcher.rs`** -- `SkillWatcher` using `notify-debouncer-full` for filesystem change events on SKILL.md files.
- **`loader.rs`** -- Legacy single-file skill parser (internal only, will be removed).

## SKILL.md Format

Skills are defined as directories containing a `SKILL.md` file. The file uses YAML frontmatter delimited by `---` followed by a Markdown body with LLM instructions.

### Frontmatter Schema

```yaml
---
name: commit
description: "Create conventional git commits"
allowed-tools: ["Read", "Write", "Bash"]
homepage: https://example.com
license: MIT
compatibility: "claude-code"
dockerfile: Dockerfile
requires:
  bins: ["git"]
  any_bins: []
  install:
    - kind: brew
      formula: git
      bins: ["git"]
      os: ["darwin"]
    - kind: npm
      package: some-tool
      bins: ["some-tool"]
      os: ["linux"]
---

# Instructions

Write commit messages following conventional commit format...
```

### Frontmatter Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Skill name -- lowercase alphanumeric + hyphens, 1-64 chars |
| `description` | string | no | Short human-readable description |
| `allowed-tools` | list | no | Tools this skill is allowed to use |
| `homepage` | string | no | Homepage URL |
| `license` | string | no | SPDX license identifier |
| `compatibility` | string | no | Environment requirements description |
| `dockerfile` | string | no | Relative path to Dockerfile for sandbox environment |
| `requires.bins` | list | no | All of these binaries must be found in PATH |
| `requires.any_bins` | list | no | At least one of these binaries must be found |
| `requires.install` | list | no | Install instructions for missing binaries |

### Install Spec

Each install entry supports these fields:

| Field | Description |
|-------|-------------|
| `kind` | Install method: `brew`, `npm`, `go`, `cargo`, `uv`, `download` |
| `formula` | Homebrew formula name (for `brew`) |
| `package` | Package name (for `npm`, `cargo`, `uv`) |
| `module` | Go module path (for `go`) |
| `url` | Download URL (for `download`) |
| `bins` | Binaries provided by this install step |
| `os` | Platform filter, e.g. `["darwin"]`, `["linux"]`. Empty = all platforms |
| `label` | Human-readable label for the install option |

### OpenClaw Compatibility

The parser also supports OpenClaw/ClawdBot metadata format, merging requirements from `metadata.openclaw.requires`, `metadata.clawdbot.requires`, or `metadata.moltbot.requires` when top-level `requires` is empty.

## Discovery Sources

Skills are discovered from four sources, in priority order:

| Source | Path | Description |
|--------|------|-------------|
| Project | `<data_dir>/.rara/skills/` | Project-local skills |
| Personal | `<data_dir>/skills/` | User's personal skills |
| Registry | `<data_dir>/installed-skills/` | Installed from GitHub repos |
| Plugin | `<data_dir>/installed-plugins/` | Scanned from ClaudeCode-style plugin directories |

Project and personal skills are scanned one level deep for directories containing `SKILL.md`. Registry and plugin skills use a JSON manifest to track enabled/trusted state.

## Usage Example

```rust
use rara_skills::{discover::FsSkillDiscoverer, registry::InMemoryRegistry};

// Build discoverer with default search paths
let paths = FsSkillDiscoverer::default_paths();
let discoverer = FsSkillDiscoverer::new(paths);

// Populate registry from discovered skills
let registry = InMemoryRegistry::from_discoverer(&discoverer).await?;
let skills = registry.list_all();

// Generate system prompt injection
let prompt = rara_skills::prompt_gen::generate_skills_prompt(&skills);
```

### Installing from GitHub

```rust
use rara_skills::install;

let install_dir = install::default_install_dir()?;
let skills = install::install_skill("owner/repo", &install_dir).await?;
```

### Checking Requirements

```rust
use rara_skills::requirements::check_requirements;

let eligibility = check_requirements(&skill_metadata);
if !eligibility.eligible {
    println!("Missing: {:?}", eligibility.missing_bins);
    for opt in &eligibility.install_options {
        let preview = rara_skills::requirements::install_command_preview(opt)?;
        println!("  Fix: {preview}");
    }
}
```

### Watching for Changes

```rust
use rara_skills::watcher::SkillWatcher;

let dirs = vec![data_dir.join(".rara/skills"), data_dir.join("skills")];
let (_watcher, mut rx) = SkillWatcher::start(dirs)?;

while let Some(event) = rx.recv().await {
    // Re-discover skills on change
}
```
