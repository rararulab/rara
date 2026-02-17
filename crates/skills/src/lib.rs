//! Skill discovery, management, and prompt-injection library.
//!
//! Skills are defined as `SKILL.md` files with YAML frontmatter and a Markdown
//! body. This crate handles discovering skills from multiple sources
//! (project-local, personal, plugins, registry), parsing their metadata,
//! checking binary requirements, installing from GitHub repos, and generating
//! XML prompt blocks for LLM system prompt injection.

pub mod cache;
pub mod discover;
pub mod error;
pub mod formats;
pub mod hash;
pub mod install;
pub mod manifest;
pub mod parse;
pub mod prompt_gen;
pub mod registry;
pub mod requirements;
pub mod types;
pub mod watcher;
