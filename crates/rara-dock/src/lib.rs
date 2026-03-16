//! Dock — backend foundation for the Dock generative UI.
//!
//! Provides data models, file-based persistence, prompt-building utilities,
//! agent tools, and HTTP routes for the Dock canvas workbench.

pub mod error;
pub mod models;
pub mod routes;
pub mod state;
pub mod store;
pub mod tools;

pub use error::DockError;
pub use models::*;
pub use routes::{DockRouterState, dock_router};
pub use state::{
    apply_mutation, build_dock_system_prompt, build_dock_user_prompt, next_block_id, next_fact_id,
    text_of_html,
};
pub use store::DockSessionStore;
pub use tools::{dock_tool_names, dock_tools};
