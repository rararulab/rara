# Core

`core` is the AI kernel layer of the project. It contains stable, reusable,
business-agnostic abstractions.

## Responsibilities

- Define core agent capabilities such as the execution loop, message flow, and tool protocol.
- Define shared traits/models/errors across implementations (for example `Tool`, `Runner`, and memory interfaces).
- Provide a minimal AI foundation that does not depend on domain-specific business logic.

## What Belongs Here

- `agent-core`: agent loop and plan/act/observe flow.
- `tool-core`: tool trait, registry, and tool metadata.
- `prompt-core`: prompt composition and context-window strategies.
- `memory-core` (optional): unified memory interfaces without binding to a specific vector store.

## What Does Not Belong Here

- External system adapters (MCP, OpenRouter, databases, object storage).
- Business-domain logic (jobs, resumes, scheduling, etc.).
- Application wiring concerns (API routes, worker bootstrapping, runtime composition).

## Dependency Rules

- May depend on general-purpose foundations (`serde`, `tokio`, `tracing`, etc.).
- Must not depend on `integrations`, `extensions`, or `runtime`.

Design goal: `core` should still represent the full "AI product essence" on its own.
