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

//! I/O — transport primitives for inbound and outbound communication.
//!
//! This module implements the kernel's I/O transport layer:
//!
//! - **Ingress**: channel adapters publish messages through
//!   [`IngressPipeline`](ingress::IngressPipeline) into the unified
//!   [`EventQueue`](crate::event_queue::EventQueue).
//! - **Egress**: the kernel event loop delivers outbound envelopes via
//!   [`Egress::deliver`](egress::Egress::deliver) to registered adapters.
//! - **Streaming**: ephemeral real-time events (token deltas, tool progress)
//!   flow through the [`StreamHub`](stream::StreamHub) for connected frontends.
//!
//! ## Architecture
//!
//! ```text
//! Adapters → IngressPipeline → EventQueue → Kernel Event Loop
//!                                                   ↓
//!                                         Egress::deliver + StreamHub
//!                                                   ↓
//!                                         Channel Adapters (Web, Telegram, ...)
//! ```

pub mod bus;
pub mod egress;
pub mod ingress;
pub mod memory_bus;
pub mod pipe;
pub mod stream;
pub mod types;
