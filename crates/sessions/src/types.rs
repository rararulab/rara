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

//! Core types for the sessions crate.
//!
//! Chat message types ([`ChatMessage`], [`MessageRole`], [`MessageContent`],
//! [`ContentBlock`], [`ToolCall`]) are re-exported from `rara-kernel` which
//! is the single canonical source of truth.
//!
//! Session-specific types ([`SessionKey`], [`SessionEntry`],
//! [`ChannelBinding`]) are also re-exported from `rara-kernel::session`.

// ---------------------------------------------------------------------------
// Re-exports from rara-kernel (canonical ChatMessage types)
// ---------------------------------------------------------------------------
pub use rara_kernel::channel::types::{
    ChatMessage, ContentBlock, MessageContent, MessageRole, ToolCall,
};
// ---------------------------------------------------------------------------
// Re-exports from rara-kernel::session (canonical session types)
// ---------------------------------------------------------------------------
pub use rara_kernel::session::{ChannelBinding, SessionEntry, SessionKey};
