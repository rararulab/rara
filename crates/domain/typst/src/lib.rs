//! # rara-domain-typst
//!
//! Typst compilation service — project management, file editing, and PDF rendering.

use std::sync::Arc;

use opendal::Operator;
use sqlx::PgPool;

pub mod compiler;
pub mod error;
pub mod git;
pub mod pg_repository;
pub mod repository;
pub mod router;
pub mod service;
pub mod types;

/// Wire the [`TypstService`](service::TypstService) with a PostgreSQL
/// repository and an S3-compatible object store.
#[must_use]
pub fn wire_typst_service(pool: PgPool, object_store: Operator) -> service::TypstService {
    let repo = Arc::new(pg_repository::PgTypstRepository::new(pool));
    service::TypstService::new(repo, object_store)
}
