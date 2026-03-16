# scripts (devtool) — Agent Guidelines

## Purpose
Unified Go CLI developer toolkit for rara. Houses automation scripts that are too complex for inline bash in the justfile.

## Architecture
Follows the [k9s](https://github.com/derailed/k9s) project structure pattern:

- `cmd/devtool/main.go` — thin entry point, wires top-level command groups
- `internal/<domain>/` — each command group gets its own package under `internal/`
  - `internal/worktree/git.go` — low-level git worktree operations
  - `internal/worktree/commands.go` — CLI subcommand definitions for `devtool wt`

CLI framework: [urfave/cli v3](https://github.com/urfave/cli)

Build: `cd scripts && go build -o bin/devtool ./cmd/devtool/`
The justfile wraps this via `just devtool-build`.

## Critical Invariants
- Go module lives in `scripts/go.mod` (not the repo root) — all `go` commands must run from `scripts/`.
- Built binary goes to `scripts/bin/` which is gitignored.
- Each new command group should be a separate package under `internal/` with its own exported `*Cmd()` functions, registered in `cmd/devtool/main.go`.

## What NOT To Do
- Do NOT put complex bash logic in the justfile — write Go instead.
- Do NOT use cobra — this project uses urfave/cli v3.
- Do NOT put business logic in `cmd/devtool/main.go` — keep it as a thin wiring layer.
- Do NOT create standalone Go binaries for each tool — add subcommands to devtool.

## Dependencies
- Upstream: justfile invokes devtool via `just wt-*` recipes.
- External: git CLI (called via `os/exec`).
