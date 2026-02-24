//! # rara-backend-admin
//!
//! Unified HTTP admin routes for all backend subsystems: settings, prompts,
//! models, dispatcher, MCP servers, skills, pipeline, and coding tasks.

pub mod coding_task;
pub mod dispatcher;
pub mod mcp;
pub mod models;
pub mod pipeline;
pub mod prompts;
pub mod settings;
pub mod skills;
