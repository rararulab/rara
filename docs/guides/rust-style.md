# Rust Code Style

## Style Direction

Write Rust in the style region defined by BurntSushi, dtolnay, and Niko Matsakis (see CLAUDE.md
for what to take from each). This means: functional-first, iterator chains over imperative loops,
combinators on Option/Result for simple transforms, `match` for complex branching, immutable by
default, early returns with `?` to keep the happy path flat.

If you're unsure whether a pattern fits, ask: "Would BurntSushi write it this way in `ripgrep`?"
If yes, it's probably right. If it feels like enterprise Java, it's probably wrong.

## Toolchain Constraints

These are zero-ambiguity rules — not style preferences, but mechanical requirements.

### Error Handling
- `snafu` exclusively in domain/kernel — never `thiserror` or manual `impl Error`
- `anyhow` allowed only at application boundaries (tool implementations, integrations, bootstrap)
- Error enum pattern: `#[derive(Debug, Snafu)]` + `#[snafu(visibility(pub))]`
- Naming: `{CrateName}Error`, variants use `#[snafu(display("..."))]`
- Propagation: `.context(XxxSnafu)?` or `.whatever_context("msg")?`
- Per-crate alias: `pub type Result<T> = std::result::Result<T, CrateError>`

### Struct Construction — `bon::Builder`
- 3+ fields: `#[derive(bon::Builder)]` — no manual `fn new()` constructors
- Config structs: pair with `Deserialize`, never `#[derive(Default)]` — defaults come from YAML
- Cross-module: `Foo::builder().field(val).build()`, not struct literals
- Within the defining module: struct literals are fine
- `Option<T>` fields auto-default to `None` in bon — no need for `#[builder(default)]`
- 1-2 field structs: direct construction, no builder needed

### Type Patterns
- Trait objects: `pub type XRef = Arc<dyn X>` alias
- No hardcoded config defaults in Rust — all via YAML config file

### Async
- `#[async_trait]` + `Send + Sync` bound on async trait definitions
- Logging: `tracing` macros + `#[instrument(skip_all)]`

### Code Organization
- `mod.rs` only for re-exports + `//!` module docs — split logic into sub-files
- Imports: `std` → external crates → internal (`crate::` / `super::`)
- No wildcard imports (`use foo::*`)
- `.expect("context")` over `unwrap()` in non-test code
