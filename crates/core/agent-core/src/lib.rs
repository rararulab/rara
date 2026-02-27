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

//! Thin re-export facade — all modules have been moved to `rara-kernel`.
//!
//! This crate exists only for backward compatibility. New code should import
//! directly from `rara_kernel`.

// Re-export everything from rara-kernel under the old agent-core module names.
pub use rara_kernel::agent_context as context;
pub use rara_kernel::memory;
pub use rara_kernel::model;
pub use rara_kernel::model_repo;
pub use rara_kernel::prompt;
pub use rara_kernel::provider;
pub use rara_kernel::runner;
pub use rara_kernel::subagent;
pub use rara_kernel::tool as tool_registry;

/// Backward-compatible error module.
///
/// Maps old `agent_core::err` paths to `rara_kernel::error`.
pub mod err {
    pub use rara_kernel::error::*;

    // The old code used `err::Error` everywhere; map to `KernelError`.
    pub type Error = rara_kernel::error::KernelError;
    pub type Result<T> = std::result::Result<T, Error>;

    pub mod prelude {
        pub use super::*;
    }
}
