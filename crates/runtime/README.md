# Runtime

`runtime` is the final composition layer (composition root). It assembles
`core + integrations + extensions` into runnable applications.

## Responsibilities

- Read configuration and initialize dependencies (DB, queue, LLM, MCP, etc.).
- Select and load extensions (feature flags or configuration-driven).
- Inject tool registries and start API / Worker / CLI processes.

## What Belongs Here

- Application entrypoints.
- Dependency injection and lifecycle management.
- Multi-module composition strategy and runtime switches.

## What Does Not Belong Here

- Business rule implementations (these belong in `extensions`).
- Third-party SDK encapsulation details (these belong in `integrations`).
- Core protocol definitions (these belong in `core`).

## Dependency Rules

- May depend on `core`, `integrations`, and `extensions`.
- Other layers must not depend back on `runtime`.

Design goal: one shared core + extension set should support multiple runtime
shapes (API service, background worker, local CLI).
