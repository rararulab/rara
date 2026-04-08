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

//! Structured, evolvable persona system for rara agents.
//!
//! The soul framework provides:
//! - **Soul files**: Markdown documents with YAML frontmatter defining an
//!   agent's personality, boundaries, and behavior guide.
//! - **Soul state**: Runtime state tracking mood, relationship stage, emerged
//!   traits, and style drift.
//! - **Rendering**: Combining soul file + state into a final prompt string.
//! - **Loading**: Priority-chain loading (per-agent file > global file > code
//!   default).
//! - **Evolution**: Snapshot management and boundary validation for soul
//!   evolution (LLM-driven rewrite deferred to future work).
//!
//! # Default Soul Definitions
//!
//! Hardcoded defaults for rara and nana are available via [`defaults`].
//! These serve as code fallbacks when no user-defined soul file exists.

pub mod defaults;
pub mod error;
pub mod evolution;
pub mod file;
pub mod loader;
pub mod render;
pub mod score;
pub mod state;

// Re-export key types for convenience.
pub use error::SoulError;
pub use evolution::{create_snapshot, list_snapshots, load_snapshot, validate_boundaries};
pub use file::{Boundaries, EvolutionConfig, SoulFile, SoulFrontmatter};
pub use loader::{LoadedSoul, SoulSource, load_and_render, load_soul, soul_path};
pub use render::render;
pub use score::{StyleScore, StyleScoreError};
pub use state::{MoodLabel, RelationshipStage, SoulState, StyleDrift};
