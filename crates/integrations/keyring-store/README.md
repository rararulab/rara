# rara-keyring-store

Thin abstraction over the OS credential store (macOS Keychain, Linux Secret Service, Windows Credential Manager) for persisting secrets such as OAuth tokens and API keys.

## Overview

The crate exposes a `KeyringStore` trait with three operations — `load`, `save`, and `delete` — each addressed by a `(service, account)` pair. `DefaultKeyringStore` implements the trait by delegating to the [`keyring`](https://crates.io/crates/keyring) crate with platform-native backends:

| Platform | Backend |
|----------|---------|
| macOS | Security Framework (`apple-native`) |
| Linux | Secret Service via `linux-native-async-persistent` |
| Windows | Windows Credential Manager (default) |

## Usage

```rust
use rara_keyring_store::{DefaultKeyringStore, KeyringStore};

let store = DefaultKeyringStore;

// Save a credential
store.save("my-app", "oauth-token", "secret-value")?;

// Load it back
let token = store.load("my-app", "oauth-token")?;
assert_eq!(token, Some("secret-value".to_string()));

// Delete when no longer needed
let removed = store.delete("my-app", "oauth-token")?;
assert!(removed);
```

## Design Decisions

- **Trait-based** — consumers depend on `KeyringStore`, making it easy to swap in a test double or an alternative backend.
- **`NoEntry` is not an error** — `load` returns `Ok(None)` and `delete` returns `Ok(false)` when the entry doesn't exist, matching the "query" semantics callers expect.
- **Structured tracing** — every method is annotated with `#[tracing::instrument]` at debug level, giving callers structured logs (service, account, value_len) for free without leaking secret values.
- **snafu errors** — all keyring failures are wrapped in a single `Error::Keyring` variant with automatic source-location tracking.
