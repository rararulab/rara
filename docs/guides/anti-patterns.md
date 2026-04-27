# What NOT To Do

Every entry has a **why** — the reasoning generalizes better than the rule alone.

## Code & Architecture

- Do NOT put repository impls or routes in `yunara-store` — **why:** business logic mixed into the store layer creates circular dependencies and makes testing impossible without the full store stack
- Do NOT use manual `impl Display` + `impl Error` — **why:** `snafu` generates consistent, composable error types; hand-rolled impls drift in style and miss context propagation
- Do NOT use mock repositories in tests — **why:** mock/prod divergence masked a broken migration (historical incident); `testcontainers` catches real DB behavior
- Do NOT use noop/hollow trait implementations (silently return `Ok(())` / `Ok(None)` / `vec![]`) — **why:** silent success hides integration bugs; if nothing tests or calls a method's return value, the method shouldn't exist. Exception: optional UX hooks (`typing_indicator`, lifecycle hooks) where no-op is the correct default
- Do NOT construct hollow `Principal` objects — **why:** placeholder values bypass permission checks; `Principal` must come from `SecuritySubsystem::resolve_principal()` or `Principal::from_user()` with real role + permissions from the database
- Do NOT write manual `fn new()` for 3+ field structs — **why:** `bon::Builder` provides consistent, IDE-friendly construction; manual constructors create positional-argument bugs
- Do NOT hardcode database URLs or config defaults in Rust — **why:** config must be explicit and auditable in YAML; hidden defaults cause "works on my machine" failures
- Do NOT expose mechanism-tuning constants as required YAML — **why:** ring-buffer caps, sweeper intervals, retry backoffs, and similar internal knobs have no deployment-relevant "right" value. They belong as `const` next to the mechanism they tune. A YAML knob recreates the original footgun (#1804 → #1817 → #1831 → #1882) where every default config silently disables or misconfigures the fix. Test: "would a deploy operator have a real reason to pick a different value?" If no → `const`.
- Do NOT modify already-applied migration files — **why:** SQLx tracks checksums; any change breaks startup on every deployed instance
- Do NOT write code comments in any language other than English — **why:** non-English comments fragment search and break tooling for international contributors
- Do NOT enable continuation for worker/child agents — **why:** child agents run in bounded context; self-continuation would break the parent's timeout and resource accounting

## Workflow

- Do NOT work directly on `main` — **why:** direct commits bypass CI, review, and issue tracking; even one-line changes need the safety net
- Do NOT merge locally — **why:** local merges skip CI checks and lose the PR audit trail
- Do NOT edit files in the main checkout for 'quick fixes' — **why:** this is the same rule as above, stated explicitly because "just this once" is the most common failure mode
- Do NOT create issues/PRs without proper labels (agent + type + component) — **why:** unlabeled items break automated dashboards and make triage impossible
- Do NOT leave stale worktrees — **why:** stale worktrees accumulate disk usage and cause branch confusion
- Do NOT report PR as complete before CI is green — **why:** user acts on "done" signal; reporting prematurely wastes their time when CI fails
- Do NOT create a new crate without an `AGENT.md` — **why:** without agent guidelines, the next agent working in this crate will repeat the same mistakes

## Agent System Prompt

- Do NOT add "plan before act" rules — **why:** causes redundant narrative even for simple "hello" interactions; the correct principle is "act first, report after" (see #201)
- Do NOT use overly broad memory search triggers — **why:** "proactively search memory" fires on every interaction, producing meaningless narrative; scope triggers explicitly (e.g., "user explicitly asks about past events")
- Do NOT modify agent system prompts without testing — **why:** prompt changes have non-obvious emergent effects; verify with simple inputs ("hello" / "你好") that no abnormal output is produced
