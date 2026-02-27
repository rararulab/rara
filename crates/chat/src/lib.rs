// Copyright 2025 Crrow
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

//! # rara-domain-chat
//!
//! Chat domain crate — orchestrates session-based AI conversations.
//!
//! This crate sits between the HTTP transport layer and the lower-level
//! [`rara_sessions`] (persistence) and [`rara_kernel`] (LLM execution)
//! crates. It is responsible for:
//!
//! - Managing the lifecycle of chat sessions (create, list, get, delete).
//! - Persisting user and assistant messages.
//! - Invoking the [`AgentRunner`](rara_kernel::runner::AgentRunner) with the
//!   session's conversation history and tool registry.
//! - Forking sessions to explore alternative conversation branches.
//! - Mapping external messaging channels to internal sessions via channel
//!   bindings.
//!
//! ## HTTP API
//!
//! HTTP routes are defined in `rara-backend-admin::chat` and expose all
//! endpoints under `/api/v1/chat/`.

pub mod agent;
pub mod error;
pub mod message_utils;
pub mod model_catalog;
pub mod service;
pub mod stream;
