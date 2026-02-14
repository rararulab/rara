//! # rara-domain-chat
//!
//! Chat domain crate — orchestrates session-based AI conversations.
//!
//! This crate sits between the HTTP transport layer and the lower-level
//! [`rara_sessions`] (persistence) and [`rara_agents`] (LLM execution)
//! crates. It is responsible for:
//!
//! - Managing the lifecycle of chat sessions (create, list, get, delete).
//! - Persisting user and assistant messages.
//! - Invoking the [`AgentRunner`](rara_agents::runner::AgentRunner) with
//!   the session's conversation history and tool registry.
//! - Forking sessions to explore alternative conversation branches.
//! - Mapping external messaging channels to internal sessions via channel
//!   bindings.
//!
//! ## HTTP API
//!
//! The [`router`] module exposes all endpoints under `/api/v1/chat/`.
//! See [`router::routes`] for the full route table.

pub mod error;
pub mod router;
pub mod service;
