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

mod router;
pub mod error;
pub mod pg_repository;
pub mod repository;
pub mod service;
pub mod state_machine;
pub mod types;

pub use router::routes;

/// Wire up the application service with a PostgreSQL repository.
#[must_use]
pub fn wire(pool: sqlx::PgPool) -> service::ApplicationService {
    let repo: std::sync::Arc<dyn repository::ApplicationRepository> =
        std::sync::Arc::new(pg_repository::PgApplicationRepository::new(pool));
    service::ApplicationService::new(repo)
}
