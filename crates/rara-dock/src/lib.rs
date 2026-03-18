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

//! Dock — backend foundation for the Dock generative UI.
//!
//! Provides data models, file-based persistence, prompt-building utilities,
//! agent tools, and HTTP routes for the Dock canvas workbench.

pub mod error;
pub mod models;
pub mod routes;
pub mod state;
pub mod store;
// Re-export `rara_kernel::tool` so the `ToolDef` proc macro can resolve
// `crate::tool::AgentTool` in derived impls.
pub use rara_kernel::tool;
pub mod tools;

pub use error::DockError;
pub use models::*;
pub use routes::{DockRouterState, dock_router};
pub use state::{
    apply_mutation, build_dock_system_prompt, build_dock_user_prompt, next_block_id, next_fact_id,
    text_of_html,
};
pub use store::DockSessionStore;
pub use tools::{DockMutationSink, dock_tool_names, dock_tools};
