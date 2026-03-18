# rara-soul — Agent Guidelines

## Purpose

Structured, evolvable persona system — manages agent personality definitions (soul files), runtime state tracking, prompt rendering, and soul evolution with boundary validation.

## Architecture

### Key modules

- `src/file.rs` — `SoulFile` (markdown with YAML frontmatter), `SoulFrontmatter` (name, version, boundaries, evolution config), `Boundaries` (immutable traits, formality range).
- `src/state.rs` — `SoulState` (runtime mood, relationship stage, emerged traits, style drift, discovered interests). Persisted as YAML alongside the soul file.
- `src/loader.rs` — `load_soul()` priority-chain: per-agent file (`~/.config/rara/souls/<agent>.md`) > code defaults. `load_and_render()` combines soul + state into final prompt.
- `src/render.rs` — `render()` merges soul file body + state context into the prompt string injected into agent turns.
- `src/evolution.rs` — `validate_boundaries()` checks proposed soul changes respect immutable traits and formality bounds. `create_snapshot()` / `load_snapshot()` / `list_snapshots()` for version history.
- `src/defaults/` — Hardcoded default soul definitions for `rara` and `nana`.
- `src/error.rs` — `SoulError` via `snafu`.

### Data flow

1. At each agent turn, kernel calls `load_and_render(agent_name)`.
2. Loader finds the soul file (user-defined or code default) and the state YAML.
3. Renderer combines soul body + serialized state into a prompt string.
4. Mita (background agent) calls `update-soul-state` / `evolve-soul` tools to modify state/soul files.
5. Evolution validates boundaries before writing, creates a snapshot of the old version.

### File layout on disk

```
~/.config/rara/souls/
  rara.md          # Soul file (YAML frontmatter + markdown body)
  rara.state.yaml  # Runtime state
  rara.snapshots/  # Version history
    v1.md
    v2.md
```

## Critical Invariants

- `immutable_traits` in `Boundaries` must never be removed or contradicted during evolution — `validate_boundaries()` enforces this.
- Formality values must stay within `min_formality..=max_formality` bounds.
- Soul file version is bumped on every evolution — never reuse a version number.
- Snapshots are append-only — never delete or modify existing snapshots.

## What NOT To Do

- Do NOT bypass `validate_boundaries()` when evolving a soul — it exists to prevent personality drift past configured limits.
- Do NOT store runtime state in the soul file itself — use the separate `.state.yaml` file.
- Do NOT trigger soul evolution frequently — Mita should evolve at most once every few days.
- Do NOT modify soul defaults in code without testing with basic conversational inputs.

## Dependencies

**Upstream:** `rara-paths` (soul file directory resolution), `serde_yaml` (frontmatter/state parsing), `jiff` (timestamps).

**Downstream:** `rara-kernel` (calls `load_and_render` for each agent turn), `rara-app` (mita tools invoke evolution).
