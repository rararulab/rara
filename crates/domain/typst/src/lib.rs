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

//! # rara-domain-typst
//!
//! Typst compilation service — project management, local filesystem file
//! editing, and PDF rendering.

use std::sync::Arc;

use opendal::Operator;
use sqlx::PgPool;

pub mod compiler;
pub mod error;
pub mod fs;
pub mod git;
pub mod pg_repository;
pub mod repository;
pub mod router;
pub mod runner;
pub mod service;
pub mod types;

/// Wire the [`TypstService`](service::TypstService) with a PostgreSQL
/// repository and an S3-compatible object store.
#[must_use]
pub fn wire_typst_service(pool: PgPool, object_store: Operator) -> service::TypstService {
    let repo = Arc::new(pg_repository::PgTypstRepository::new(pool));
    service::TypstService::new(repo, object_store)
}
