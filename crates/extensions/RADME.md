# Extensions

`extensions` is the business-capability layer. Each extension should plug into
the AI kernel as a modular, optional capability.

## Responsibilities

- Hold domain-specific capabilities (jobs, resumes, Typst, scheduling, etc.).
- Expose business capabilities to the agent as tools or services.
- Provide a consistent registration entrypoint (for example `register_tools(...)`).

## What Belongs Here

- `ext-job`: job crawling/analysis capabilities.
- `ext-resume`: resume parsing, matching, and optimization capabilities.
- `ext-typst`: document project and compilation capabilities.
- `ext-scheduler`: scheduling and trigger capabilities.

## What Does Not Belong Here

- Core abstractions such as the agent loop and tool trait.
- Third-party protocol adapter details (MCP, OpenRouter SDK, etc.).
- Application-level startup/wiring logic for API or workers.

## Dependency Rules

- May depend on `core`, and optionally on selected `integrations`.
- Must not depend on `runtime`.
- Extensions should avoid depending on each other to prevent a dependency web.

## Organization Guidelines

- One extension per crate, with explicit boundaries.
- Expose a minimal public interface; keep internals private.
- Keep tool names and schemas stable for long-term agent reuse.
