//! # rara-sessions
//!
//! Pure storage layer for AI agent conversation sessions.
//!
//! This crate provides session persistence backed by PostgreSQL, with no
//! knowledge of agents, LLM providers, or application logic. It is designed
//! to be consumed by higher-level orchestration layers (e.g. the chat domain
//! crate) that compose session storage with agent execution.
//!
//! ## Core concepts
//!
//! - **[`SessionEntry`](types::SessionEntry)** — Metadata for a single
//!   conversation (key, title, model, message count, etc.).
//! - **[`ChatMessage`](types::ChatMessage)** — An individual message within a
//!   session, tagged by [`MessageRole`](types::MessageRole) and ordered by a
//!   monotonically increasing sequence number.
//! - **[`ChannelBinding`](types::ChannelBinding)** — Maps an external channel
//!   (Telegram chat, Slack channel, etc.) to a session key so that incoming
//!   messages can be routed to the correct conversation.
//! - **[`SessionRepository`](repository::SessionRepository)** — Async trait
//!   defining the persistence contract for sessions, messages, and bindings.
//! - **[`PgSessionRepository`](pg_repository::PgSessionRepository)** —
//!   PostgreSQL implementation of the repository trait.
//!
//! ## Feature: session forking
//!
//! Sessions can be *forked* at a specific message sequence number. This copies
//! all messages up to and including the fork point into a new session, allowing
//! exploration of alternative conversation branches without losing the
//! original history.

pub mod error;
pub mod pg_repository;
pub mod repository;
pub mod store;
pub mod types;
