# What NOT To Do

## Code & Architecture
- Do NOT put repository impls or routes in `yunara-store` — business logic stays in its own crates
- Do NOT use manual `impl Display` + `impl Error` — use `snafu`
- Do NOT use mock repositories in tests — use `testcontainers`
- Do NOT use noop/hollow trait implementations — trait methods with real implementations must not have default empty bodies (silently return `Ok(())` / `Ok(None)` / `vec![]`). Optional UX hooks (`typing_indicator`, lifecycle hooks) are the only exception
- Do NOT construct hollow identity objects — `Principal` must be built via `SecuritySubsystem::resolve_principal()` or `Principal::from_user()` with full role + permissions from the database. Never store placeholder values in Session
- Do NOT write manual `fn new()` constructors for structs with 3+ fields — use `#[derive(bon::Builder)]` and construct via `Foo::builder().field(val).build()`. Config structs must also derive `Deserialize` and never `#[derive(Default)]`
- Do NOT hardcode database URLs or config defaults in Rust code — use the YAML config file
- Do NOT modify already-applied migration files — create a new migration instead
- Do NOT write code comments in any language other than English

## Workflow
- Do NOT work directly on `main` — ALL changes (code, docs, config) require a worktree + PR, no exceptions
- Do NOT merge locally on `main` — all merges go through GitHub PRs; never `git merge` or `git commit` on main
- Do NOT edit files in the main checkout for 'quick fixes' — even one-line changes must go through the full issue → worktree → PR flow
- Do NOT create issues without `created-by:claude` label
- Do NOT create PRs or issues without type + component labels — every PR and issue must have a type label (`bug`, `enhancement`, `refactor`, `chore`, `documentation`) and a component label (`core`, `backend`, `ui`, `extension`, `ci`)
- Do NOT leave stale worktrees — clean up after PR is merged
- Do NOT report PR as complete before CI is green — use `gh pr checks --watch` and fix failures before reporting
- Do NOT create a new crate without an `AGENT.md` — every crate must ship with agent guidelines from day one

## Agent System Prompt
- Do NOT add "plan before act" rules to agent system prompts — this causes redundant/repetitive narrative text even for simple interactions (hello). The correct principle is "act first, report after" (see #201)
- Do NOT use overly broad conditions to trigger memory search — "proactively search memory" causes every interaction to trigger search + meaningless narrative. Trigger conditions must be explicitly scoped (e.g., "user explicitly asks about past events")
- Do NOT modify agent system prompts without testing — at minimum, verify with simple inputs like "hello" / "你好" that no abnormal/repetitive output is produced
