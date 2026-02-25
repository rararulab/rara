//! # rara-backend-admin
//!
//! Unified HTTP admin routes for all backend subsystems: settings, prompts,
//! models, dispatcher, MCP servers, skills, pipeline, coding tasks, and
//! all domain routes (resume, application, interview, scheduler, analytics,
//! job, chat).

pub mod analytics;
pub mod application;
pub mod chat;
pub mod contacts;
pub mod coding_task;
pub mod dispatcher;
pub mod interview;
pub mod job;
pub mod mcp;
pub mod models;
pub mod pipeline;
pub mod prompts;
pub mod resume;
pub mod scheduler;
pub mod settings;
pub mod skills;
