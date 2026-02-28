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

//! Channel abstraction layer.
//!
//! Provides a unified interface for communication across different platforms
//! (Web, Telegram, CLI, internal schedulers, etc.). Inspired by
//! [openfang-channels](https://github.com/RightNow-AI/openfang/tree/main/crates/openfang-channels).
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────┐   ┌──────────┐   ┌──────────┐
//! │ Telegram  │   │   Web    │   │   CLI    │  ... adapters
//! │ Adapter   │   │ Adapter  │   │ Adapter  │
//! └────┬──---─┘   └────┬─────┘   └────┬─────┘
//!      │               │              │
//!      ▼               ▼              ▼
//!  ┌──────────────────────────────────────┐
//!  │         InboundSink (I/O Bus)       │  ingest()
//!  │   (identity, session, bus)          │
//!  └────────────────┬─────────────────────┘
//!                   │
//!                   ▼
//!              Agent Execution
//! ```
//!
//! ## Core Types
//!
//! - [`ChannelMessage`](types::ChannelMessage) — unified inbound message
//! - [`ChannelType`](types::ChannelType) — platform identifier
//! - [`AgentPhase`](types::AgentPhase) — lifecycle phase for UX feedback
//!
//! ## Core Traits
//!
//! - [`ChannelAdapter`](adapter::ChannelAdapter) — platform adapter interface

pub mod adapter;
pub mod command;
pub mod types;
