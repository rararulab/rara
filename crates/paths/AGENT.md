# rara-paths — Agent Guidelines

## Purpose

Centralized path resolution for all rara data, config, cache, and log directories — provides platform-aware static accessors that are computed once and cached via `OnceLock`.

## Architecture

### Key module

- `src/lib.rs` — The entire crate. All functions are `pub fn name() -> &'static PathBuf` backed by `OnceLock<PathBuf>` statics.

### Directory layout (macOS)

| Function | Path |
|---|---|
| `config_dir()` | `~/.config/rara` |
| `config_file()` | `~/.config/rara/config.yaml` |
| `data_dir()` | `~/Library/Application Support/rara` |
| `database_dir()` | `<data_dir>/db` |
| `sessions_dir()` | `<data_dir>/sessions` |
| `memory_dir()` | `<data_dir>/memory` |
| `resources_dir()` | `<data_dir>/resources` |
| `images_dir()` | `<data_dir>/resources/images` |
| `skills_dir()` | `<config_dir>/skills` |
| `prompts_dir()` | `<config_dir>/prompts` |
| `workspace_dir()` | `<config_dir>/workspace` |
| `staging_dir()` | `<data_dir>/staging` |
| `temp_dir()` | `~/Library/Caches/rara` |
| `logs_dir()` | `~/Library/Logs/rara` |
| `log_file()` | `<logs_dir>/rara.log` |

### Custom directory overrides

- `set_custom_data_dir(path)` — overrides `data_dir()` and derived directories. Must be called before any `data_dir()` / `config_dir()` access.
- `set_custom_config_dir(path)` — overrides `config_dir()` and derived directories.

## Critical Invariants

- All path functions use `OnceLock` — values are computed once and immutable thereafter.
- `set_custom_data_dir()` / `set_custom_config_dir()` must be called before any path accessor. Calling after initialization causes a panic.
- `staging_dir()` and `workspace_dir()` create the directory on first access — other functions return paths without creating them.

## What NOT To Do

- Do NOT call `set_custom_data_dir()` after any code has called `data_dir()` or `config_dir()` — it will panic.
- Do NOT hardcode paths in other crates — always use `rara_paths` accessors.
- Do NOT assume Linux directory layout on macOS or vice versa — the crate handles platform differences.
- Do NOT add `Default` implementations — paths are derived from platform conventions.

## Dependencies

**Upstream:** `dirs` (platform home/config/data directory detection).

**Downstream:** Nearly every crate in the workspace depends on `rara-paths` for file locations.
