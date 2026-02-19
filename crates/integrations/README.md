# Integrations

`integrations` is the external-adapter layer. It connects third-party protocols
and services to `core` abstractions.

## Responsibilities

- Implement interfaces defined by `core` (for example LLM providers, tool providers, and memory backends).
- Encapsulate protocol details, SDK usage, retries, and error translation.
- Expose stable adapter APIs to upper layers.

## What Belongs Here

- `mcp`: MCP client/manager, tool discovery, and tool-call adaptation.
- `llm-openrouter`: OpenRouter provider implementation.
- Other external adapters (vector stores, storage, notifications, search, etc.).

## What Does Not Belong Here

- Agent loop and tool protocol definitions (these belong in `core`).
- Business orchestration and domain rules (these belong in `extensions`).
- Application startup and final composition wiring (these belong in `runtime`).

## Dependency Rules

- May depend on `core`.
- Must not depend on `extensions` or `runtime`.

Design goal: external service swaps should not affect `core` or business extensions.
