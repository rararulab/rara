//! Dock — backend foundation for the Dock generative UI.
//!
//! Provides data models, file-based persistence, and prompt-building utilities
//! for the Dock canvas workbench.

pub mod error;
pub mod models;
pub mod state;
pub mod store;

pub use error::DockError;
pub use models::*;
pub use state::{
    apply_mutation, build_dock_system_prompt, build_dock_user_prompt, next_block_id, next_fact_id,
    text_of_html,
};
pub use store::DockSessionStore;
