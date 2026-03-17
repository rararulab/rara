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

//! Kernel notifications — two independent subsystems:
//!
//! - **Event bus** (`bus` module) — fire-and-forget broadcast of internal
//!   kernel events ([`KernelNotification`], [`NotificationBus`]).
//! - **Task subscriptions** (`subscription` module) — tag-based, owner-scoped
//!   delivery of [`TaskNotification`]s to subscribing sessions.

mod bus;
mod subscription;

// Re-export everything so `use crate::notification::X` keeps working.
pub use bus::*;
pub use subscription::*;
