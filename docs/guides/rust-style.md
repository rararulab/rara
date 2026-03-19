# Rust Code Style

## Error Handling

- Use `snafu` exclusively — never `thiserror` or manual `impl Error`
- `anyhow` allowed at application boundaries (tool implementations, integrations, app bootstrap) but NOT in domain/kernel core logic — use `snafu` there
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

## Functional Style

Prefer functional programming patterns over imperative code:

- **Iterator chains** over `for` loops with manual accumulation — use `.map()`, `.filter()`, `.flat_map()`, `.fold()`, `.collect()`
- **Early returns with `?`** over nested `if let` / `match` — keep the happy path flat
- **Combinators on Option/Result** — `.map()`, `.and_then()`, `.unwrap_or_else()`, `.ok_or_else()` over `match` when the logic is a simple transform
- **`match` for complex branching** — use `match` when there are 3+ arms or when destructuring is needed; don't force combinators into unreadable chains
- **Closures** for short inline logic; extract to named functions when the closure exceeds ~5 lines
- **Immutable by default** — only use `mut` when mutation is genuinely needed
- **`let` bindings for intermediate results** — name intermediate values to improve readability rather than chaining everything into one expression
- Avoid side effects in iterator chains — if you need side effects, use `for` or `.for_each()`

```rust
// Good: functional chain
let active_names: Vec<_> = users
    .iter()
    .filter(|u| u.is_active)
    .map(|u| &u.name)
    .collect();

// Bad: imperative accumulation
let mut active_names = Vec::new();
for u in &users {
    if u.is_active {
        active_names.push(&u.name);
    }
}

// Good: combinator on Option
let display = user.nickname.as_deref().unwrap_or(&user.name);

// Bad: match for simple default
let display = match &user.nickname {
    Some(n) => n.as_str(),
    None => &user.name,
};
```

## Code Organization

- Split logic into sub-files; `mod.rs` only for re-exports + `//!` module docs
- Imports grouped: `std` → external crates → internal (`crate::` / `super::`)
- No wildcard imports (`use foo::*`)
- All `pub` items must have `///` doc comments in English
- Use `.expect("context")` over `unwrap()` in non-test code
