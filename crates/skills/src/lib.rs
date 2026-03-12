// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

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
pub mod marketplace;
pub mod parse;
pub mod prompt_gen;
pub mod registry;
pub mod requirements;
pub mod types;
pub mod watcher;
