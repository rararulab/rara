# Guard System — Design & Constraints

> **For AI agents working on this code**: Read this before modifying the guard module.

## What This Is

A two-layer security gate between permission checks and tool execution in `agent.rs`.
Every tool call passes through `GuardPipeline::pre_execute()` before running.

```
Permission check → [Guard Pipeline] → Tool execution
                    ├── Layer 1: Taint (session-level)
                    └── Layer 2: Pattern (argument-level)
```

## Layer 1: Taint Tracking (`taint.rs`)

**Concept**: Track data provenance through sessions. When a tool brings in external data
(e.g. `web_fetch`), the session gets a taint label. Subsequent tool calls are checked
against sink policies — e.g. a tainted session cannot call `bash`.

**Key invariant**: Labels only accumulate, never shrink (except via `clear_session` on
session cleanup). This is intentional — once a session is tainted, it stays tainted.

### Source → Label mapping (`labels_for_tool_output`)

| Tool output | Label |
|-------------|-------|
| `web_fetch`, `browser_*` | `ExternalNetwork` |
| `agent_send`, `agent_spawn` | `UntrustedAgent` |

### Sink → Blocked labels (`sink_for_tool`)

| Sink tool | Blocked labels | Rationale |
|-----------|---------------|-----------|
| `bash`, `shell_exec` | ExternalNetwork, UntrustedAgent, UserInput | Prevents RCE via injection |
| `file_write`, `file_delete`, `edit`, `write` | ExternalNetwork, UntrustedAgent | Prevents disk poisoning |
| `web_fetch` (outbound) | Secret, Pii | Prevents data exfiltration |
| `agent_send`, `agent_message` | Secret | Prevents secret leaks to sub-agents |

### Session lifecycle

- `post_execute` records labels after tool success
- `fork_session` copies parent labels to child (sub-agents inherit restrictions)
- `clear_session` removes state on session cleanup
- `record_secret` manually injects `Secret` label (for env vars, etc.)

## Layer 2: Pattern Scanning (`pattern.rs`)

**Concept**: Substring match against tool arguments. Catches known dangerous patterns
regardless of taint state.

### Important limitations

- **Substring matching only** — no regex, no AST parsing. This means false positives
  are possible. When in doubt, err on the side of blocking.
- `shell_only: true` rules only fire for `bash`/`shell_exec` tools.
- Shell metachar detection is intentionally limited to `| sh`, `| bash`, `| zsh`.
  We explicitly do NOT block `` ` `` or `$(` because they are ubiquitous in normal
  shell commands and would cause unacceptable false positive rates.

### Rule categories

| Category | Severity | Examples |
|----------|----------|---------|
| `prompt_override` (InjectionMarker) | Critical | "ignore previous instructions", "you are now" |
| `shell_destructive` (Destructive) | Critical | `rm -rf`, `mkfs`, `drop table` |
| `data_exfiltration` (Exfiltration) | High | `curl -d`, `nc -e`, "exfiltrate" |
| `privilege_escalation` | High | `sudo `, `chmod 777`, `chown root` |
| `shell_metachar` (ShellMetachar) | Critical | `\| sh`, `\| bash` |

## Pipeline Execution Order (`pipeline.rs`)

1. **Taint check first** — O(1) per label, cheap. If blocked, skip pattern scan entirely.
2. **Pattern scan second** — O(rules × arg_size), more expensive.
3. Only `Critical` and `High` severity pattern matches trigger a block.

This ordering is intentional for performance. Do not reverse it.

## Constraints — Do NOT Change Without Understanding

1. **Do NOT add default `impl` for `TaintTracker`** — it must be explicitly constructed
   and wired through `Kernel::new()`.

2. **Do NOT make taint labels decrementable** — `declassify` was deliberately removed.
   Taint only accumulates within a session. If you need to "untaint", create a new session.

3. **Do NOT lower pattern severity to avoid false positives** — instead, narrow the
   pattern string or add `shell_only: true`. Severity reflects real-world risk.

4. **Do NOT add `` ` `` or `$(` back to shell metachar detection** — these were removed
   because they cause massive false positives on normal shell commands. See PR #220.

5. **Do NOT skip `post_execute` on any successful tool path** — missing taint recording
   creates a security gap where subsequent tool calls bypass taint checks.

6. **Do NOT move guard checks after tool execution** — the whole point is to block
   *before* the tool runs. `pre_execute` in `agent.rs` must happen before the tool call.

7. **Error type uses `snafu`** — project convention. Do not add manual `impl Display`
   or `impl Error`. Enum Display uses `strum::Display`.

## User Approval Flow

When a guard blocks a tool call, it does **not** silently reject. Instead it routes
through the existing `ApprovalManager` so the user can override the decision:

```
GuardPipeline::pre_execute() → Blocked
  → Build ApprovalRequest (with guard layer + reason as summary)
  → ApprovalManager::request_approval() — blocks until user responds or timeout
    → Approved  → fall through to normal tool execution
    → Denied    → emit GuardDenied notification + return error to LLM
    → TimedOut  → same as Denied
```

This means:
- Users always see WHY a tool call was blocked (via the approval prompt)
- Users can override guard decisions when they know it's safe
- If `auto_approve` is enabled in `ApprovalPolicy`, guards are effectively bypassed
- The 120s timeout prevents indefinite hangs if no user is present

## Adding a New Tool

To integrate a new tool with the guard system:

1. If the tool **produces untrusted data**: add it to `labels_for_tool_output`
2. If the tool **should be restricted**: add it to `sink_for_tool` with appropriate blocked labels
3. If there are **known dangerous argument patterns**: add a `PatternRule` to `RULES`
4. Add tests for the new mappings

## Observability

All public methods on `TaintTracker` and `GuardPipeline` are instrumented with
`#[instrument]`. Use `RUST_LOG=rara_kernel::guard=debug` to see guard decisions.
The `agent.rs` integration also emits `warn!` logs and `KernelNotification::GuardDenied`
events on blocks.
