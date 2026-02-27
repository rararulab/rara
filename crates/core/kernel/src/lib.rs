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
//! Unified agent orchestrator — coordinates agent lifecycle, 3-layer memory,
//! LLM execution, and tool dispatch.
//!
//! ## Architecture
//!
//! ```text
//!                      +------------------+
//!                      |     Kernel        |  <-- unified entry point
//!                      +--+--+--+--+------+
//!          registry ------+  |  |  +------ tools (ToolRegistry)
//!          memory (3-layer) -+  +-- llm (AgentRunner)
//! ```
//!
//! The kernel does NOT own business logic. It wires together:
//! - [`agent_core::runner::AgentRunner`] for LLM execution
//! - [`memory_core`] traits for 3-layer memory
//! - [`agent_core::tool_registry::ToolRegistry`] for tool dispatch

pub mod error;
pub mod kernel;
pub mod registry;

pub use error::{KernelError, Result};
pub use kernel::{Kernel, KernelConfig};
pub use registry::{AgentEntry, AgentManifest, AgentRegistry, AgentState};
