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

//! # rara-kernel
//!
//! Unified session orchestrator — the single core crate for session lifecycle,
//! 6-component architecture, and LLM ↔ Tool execution loop.
//!
//! ## 6 Components
//!
//! | Component   | Trait            | Purpose                          |
//! |-------------|------------------|----------------------------------|
//! | LLM         | [`LlmDriver`]    | Chat completion requests         |
//! | Tool        | [`ToolRegistry`] | Tool registration + dispatch     |
//! | Memory      | (tape-based)     | Agent memory via tape subsystem   |
//! | Session     | [`SessionIndex`] | Session metadata persistence      |
//! | Guard       | [`Guard`]        | Tool approval + output moderation |
//! | Notification Bus | [`NotificationBus`] | Inter-component notification broadcasting |

pub mod agent;
pub mod channel;
pub mod error;
pub mod event;
pub mod guard;
pub mod handle;
pub mod identity;
pub mod io;
pub mod kernel;
pub mod kv;
pub mod llm;
pub mod memory;
pub mod metrics;
pub mod mood;
pub mod notification;
pub mod plan;
pub mod proactive;
pub mod queue;
pub mod schedule;
pub mod security;
pub mod session;
pub(crate) mod syscall;
pub mod tool;
pub mod trace;

pub use error::{KernelError, Result};
