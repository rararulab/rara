pub mod discover;
pub mod error;
pub mod formats;
pub mod install;
pub mod manifest;
pub mod parse;
pub mod prompt_gen;
pub mod registry;
pub mod requirements;
pub mod types;
pub mod watcher;

// Legacy loader module -- no longer part of the public API.
// Consumers should use `parse` + `registry::InMemoryRegistry` instead.
pub(crate) mod loader;
