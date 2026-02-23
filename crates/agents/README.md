# rara-agents

Agent infrastructure crate — provides the execution loop, tool system, and agent abstractions.

## Architecture

```
rara-agents/
├── runner.rs          AgentRunner — multi-turn LLM tool-call loop
├── tool_registry.rs   ToolRegistry — typed tool registration and dispatch
├── provider/          LLM provider abstraction (OpenAI-compatible)
├── model.rs           Shared model types
├── err.rs             Crate-level errors
│
├── orchestrator/      Shared agent infrastructure
│   ├── core.rs          AgentOrchestrator — prompt assembly, tool construction, context mgmt
│   ├── prompt.rs        Soul prompt, system prompt, worker policy composition
│   ├── context.rs       ChatMessage ↔ OpenAI message conversion, token estimation
│   └── reflection.rs    Post-conversation memory learning
│
├── builtin/           Built-in agents (Rust-defined, compile-time)
│   ├── chat.rs          ChatAgent — interactive chat with memory + MCP + compaction
│   ├── proactive.rs     ProactiveAgent — autonomous activity review + actions
│   └── scheduled.rs     ScheduledAgent — executes due jobs from scheduler
│
└── subagent/          Dynamic agents (Markdown-defined, runtime)
    ├── definition.rs    AgentDefinition — parsed from YAML frontmatter markdown
    ├── executor.rs      SubagentExecutor — single/chain/parallel execution
    └── tool.rs          SubagentTool — LLM-callable tool for dispatching sub-agents
```

## Two Types of Agents

| | Built-in (`builtin/`) | Dynamic (`subagent/`) |
|---|---|---|
| **Defined in** | Rust structs | Markdown files with YAML frontmatter |
| **Created by** | Developers at compile time | AI or users at runtime |
| **Examples** | ChatAgent, ProactiveAgent | scout, planner, worker |
| **Tools** | Full registry + dynamic MCP | Filtered subset of parent tools |
| **Lifecycle** | Prepare → Execute → Post-process | Single execution via SubagentTool |

## Layers

```
Layer 0: runner.rs + tool_registry.rs    Pure execution loop
Layer 1: orchestrator/                   Shared infra (prompt, tools, memory, context)
Layer 2: builtin/ + subagent/            Concrete agent implementations
```

Built-in agents use `AgentOrchestrator` for shared concerns (prompt assembly, MCP tool discovery, memory injection). The orchestrator is the bridge between the raw loop and the agent-specific logic.

## Built-in Agent Lifecycle

Each built-in agent follows a prepare → execute → post-process pattern:

```
ChatAgent.run():
  1. Compaction check (summarize if context > 80% of model limit)
  2. Build system prompt (soul + memory profile + memory prefetch + skills)
  3. Build effective tools (static registry + dynamic MCP)
  4. Run AgentRunner loop
  5. Memory reflection (fire-and-forget background task)

ProactiveAgent.run():
  1. Build worker policy (soul + on-disk/default policy)
  2. Run AgentRunner loop with activity summary
  (Future #190: Goal/Task loading, work journal, dynamic scheduling)

ScheduledAgent.run():
  1. Build worker policy
  2. Run AgentRunner loop with job message + session history
  (Future #190: Goal/Task progress tracking)
```

The prepare/post-process hooks are intentionally minimal. Issue #190 will add:
- Goal/Task persistence and state-machine progression
- QMD (Query-Memory-Decide) automatic memory injection
- Work Journal for cross-session context recovery
- Dynamic heartbeat frequency

## Callers

Built-in agents separate **agent logic** from **I/O**:

- `ChatService` — session CRUD + calls `ChatAgent`
- `ProactiveAgentWorker` — activity collection + calls `ProactiveAgent`
- `AgentSchedulerWorker` — job scheduling + calls `ScheduledAgent`

Callers handle persistence (read history, save messages, update sessions). Agents handle intelligence (prompt, tools, loop, reflection).
