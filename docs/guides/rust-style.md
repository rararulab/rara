# Rust Code Style

## Error Handling

- Use `snafu` exclusively — never `thiserror` or manual `impl Error`
- `anyhow` only allowed in `ToolExecute::run()` return types
- Every error enum: `#[derive(Debug, Snafu)]` + `#[snafu(visibility(pub))]`
- Name: `{CrateName}Error`, variants use `#[snafu(display("..."))]`
- Propagate with `.context(XxxSnafu)?` or `.whatever_context("msg")?`
- Define `pub type Result<T> = std::result::Result<T, CrateError>` per crate

## Type Patterns

- Trait objects: always create `pub type XRef = Arc<dyn X>` alias
- Config structs: `bon::Builder` + `Deserialize`, no `#[derive(Default)]`
- No hardcoded config defaults in Rust — all via YAML

## Async

- `#[async_trait]` + `Send + Sync` bound on async trait definitions
- Logging: `tracing` macros + `#[instrument(skip_all)]`

## Code Organization

- Split logic into sub-files; `mod.rs` only for re-exports + `//!` module docs
- Imports grouped: `std` → external crates → internal (`crate::` / `super::`)
- No wildcard imports (`use foo::*`)
- All `pub` items must have `///` doc comments in English
- Use `.expect("context")` over `unwrap()` in non-test code
