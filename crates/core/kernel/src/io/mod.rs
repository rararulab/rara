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

//! I/O Bus — unified message pipeline for inbound and outbound communication.
//!
//! This module implements the kernel's I/O layer, inspired by OS I/O buses:
//!
//! - **Inbound**: channel adapters publish messages to a single-consumer queue;
//!   the kernel tick loop drains them at its own pace.
//! - **Outbound**: the kernel publishes responses to a pub/sub broadcast;
//!   multiple egress subscribers deliver to their respective channels.
//! - **Streaming**: ephemeral real-time events (token deltas, tool progress)
//!   flow through the [`StreamHub`](stream::StreamHub) for connected frontends.
//! - **Scheduling**: per-session serial execution via
//!   [`SessionScheduler`](scheduler::SessionScheduler).
//!
//! ## Architecture
//!
//! ```text
//! Adapters → InboundBus → Kernel Tick → SessionScheduler → AgentExecutor
//!                                                              ↓
//!                                              OutboundBus ← StreamHub
//!                                                  ↓
//!                                              Egress (subscribers)
//! ```

pub mod bus;
pub mod egress;
pub mod executor;
pub mod ingress;
pub mod memory_bus;
pub mod scheduler;
pub mod session_manager;
pub mod stream;
pub mod tick;
pub mod types;
