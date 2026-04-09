# rara-tool-macro — Agent Guidelines

## Purpose

Proc-macro crate that generates `AgentTool` implementations from annotated structs, eliminating hand-written JSON Schema and manual parameter deserialization.

## Architecture

Single file: `src/lib.rs`.

- `#[derive(ToolDef)]` — parses `#[tool(...)]` attributes and expands into `impl AgentTool`.
- `clean_schema()` — public function that strips noise fields (`$schema`, `title`, `definitions`) from `schemars`-generated JSON Schema and inline-resolves `$ref` pointers.

### Attribute axes

`params_schema` and `execute` are independently combinable:

| params_schema | execute | Result |
|---|---|---|
| auto (default) | auto (default) | Schema from `ToolExecute::Params`, execute bridges `ToolExecute::run` |
| `params_schema = "..."` | auto (default) | Custom schema expr, execute still via `ToolExecute::run` |
| auto (default) | `execute_fn = "..."` | Schema from `ToolExecute::Params`, execute delegates to custom fn |
| `params_schema = "..."` | `execute_fn = "..."` | Both custom, no `ToolExecute` needed |
| `manual_impl = true` | — | Only generates constants, user writes full `impl AgentTool` |

### Validation bridging

The macro auto-bridges `ToolExecute::validate` → `AgentTool::validate`:

| Mode | validate behaviour |
|---|---|
| Default (ToolExecute) | Deserialises params, calls `ToolExecute::validate(&typed)` |
| `validate_fn = "..."` | Calls user fn with `&serde_json::Value` directly |
| `execute_fn` without `validate_fn` | Omitted — trait default (no-op) applies |
| `manual_impl = true` | User writes `validate` manually if needed |

### Safety axis flags

Boolean flags generate trait method overrides (default is omitted → fail-closed `false`):

| Flag | Generated method |
|---|---|
| `read_only` | `fn is_read_only(&self, _args: &Value) -> bool { true }` |
| `destructive` | `fn is_destructive(&self, _args: &Value) -> bool { true }` |
| `concurrency_safe` | `fn is_concurrency_safe(&self, _args: &Value) -> bool { true }` |
| `user_interaction` | `fn requires_user_interaction(&self) -> bool { true }` |

Example: `#[tool(name = "read_file", description = "...", read_only, concurrency_safe)]`

## Critical Invariants

- Generated `AgentTool::parameters_schema()` output must be **semantically equivalent** to the hand-written `serde_json::json!()` it replaces. Field ordering may differ.
- `clean_schema()` must remove `$schema`, `title`, `definitions`/`$defs` and inline-resolve all `$ref` pointers. LLMs cannot follow `$ref`.
- The macro references `crate::tool::ToolExecute` and `crate::tool::ToolOutput` — it is designed to be used **inside `rara-kernel`** only.

## What NOT To Do

- Do NOT add runtime dependencies to this crate — it is a proc-macro crate; only `proc-macro2`, `quote`, `syn` are allowed as dependencies.
- Do NOT change `AgentTool` trait signature — `dyn AgentTool` and `McpToolBridge` depend on it.
- Do NOT assume `$ref` paths are more than one level deep — tool param schemas are shallow.

## Dependencies

- **Upstream**: None (proc-macro crate, no workspace crate deps).
- **Downstream**: `rara-kernel` uses `#[derive(ToolDef)]` and `rara_tool_macro::clean_schema()`.
- **Peer**: `schemars` (for `JsonSchema` derive on param structs) and `serde` (for `Deserialize`).
