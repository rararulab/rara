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

//! Skill registry initialization.

use rara_skills::registry::InMemoryRegistry;
use sqlx::PgPool;
use tracing::info;

/// Create a skill registry and spawn PostgreSQL cache background sync.
pub fn init_skill_registry(pool: PgPool) -> InMemoryRegistry {
    let registry = InMemoryRegistry::new();
    rara_skills::cache::spawn_background_sync(pool, registry.clone());
    info!("skill registry initialized with background sync");
    registry
}

/// Create an empty skill registry without background sync (for testing).
pub fn empty_skill_registry() -> InMemoryRegistry {
    InMemoryRegistry::new()
}
