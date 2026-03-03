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

//! Default trait-object factories for Kernel components.

use std::sync::Arc;

use rara_kernel::{
    defaults::{broadcast_bus::BroadcastEventBus, noop::NoopMemory, noop_guard::NoopGuard},
    guard::Guard,
    memory::Memory,
    notification::EventBus,
};

/// Default Memory implementation — `NoopMemory` (kernel layer does not persist;
/// agents access memory through tools).
pub fn default_memory() -> Arc<dyn Memory> { Arc::new(NoopMemory) }

/// Default EventBus — `BroadcastEventBus` (tokio broadcast channel).
pub fn default_event_bus() -> Arc<dyn EventBus> { Arc::new(BroadcastEventBus::default()) }

/// Default Guard — `NoopGuard` (allows all operations, no approval).
///
/// **For testing/development only.** Production code should use `PathGuard`
/// wrapping `NoopGuard` to enforce file-system sandboxing. See
/// `rara-app` for the production wiring.
pub fn default_guard() -> Arc<dyn Guard> { Arc::new(NoopGuard) }

/// Default UserStore — `NoopUserStore` (all users permitted).
///
/// **For testing only.** Production code should use
/// [`PgUserStore`](crate::user_store::PgUserStore) backed by a real
/// PostgreSQL connection pool. See `rara-app` for the production wiring.
pub fn default_user_store() -> Arc<dyn rara_kernel::process::user::UserStore> {
    Arc::new(rara_kernel::defaults::noop_user_store::NoopUserStore)
}
