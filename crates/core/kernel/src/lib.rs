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

//! # rara-kernel
//!
//! Unified agent orchestrator — the single core crate for agent lifecycle,
//! 7-component architecture, and LLM ↔ Tool execution loop.
//!
//! ## 7 Components
//!
//! | Component   | Trait            | Purpose                          |
//! |-------------|------------------|----------------------------------|
//! | LLM         | [`LlmApi`]       | Chat completion requests         |
//! | Tool        | [`ToolRegistry`] | Tool registration + dispatch     |
//! | Memory      | [`Memory`]       | 3-layer memory (State/Knowledge/Learning) |
//! | Session     | [`SessionStore`] | Conversation history persistence |
//! | Prompt      | [`PromptRepo`]   | System prompt templates          |
//! | Guard       | [`Guard`]        | Tool approval + output moderation |
//! | Event Bus   | [`EventBus`]     | Inter-component event broadcasting |

pub mod agent_context;
pub mod context;
pub mod defaults;
pub mod error;
pub mod event;
pub mod guard;
pub mod kernel;
pub mod llm;
pub mod memory;
pub mod model;
pub mod model_repo;
pub mod prompt;
pub mod provider;
pub mod registry;
pub mod runner;
pub mod session;
pub mod subagent;
pub mod tool;

pub use context::RunContext;
pub use error::{KernelError, Result};
pub use kernel::{Kernel, KernelConfig};
pub use registry::{AgentEntry, AgentManifest, AgentRegistry, AgentState, GuardPolicy};
