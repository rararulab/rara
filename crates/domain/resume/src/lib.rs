use std::sync::Arc;
use sqlx::PgPool;

pub mod pg_repository;
pub mod repository;
pub mod service;
pub mod types;

pub type ResumeAppService = service::ResumeService<pg_repository::PgResumeRepository>;

#[must_use]
pub fn wire_resume_service(pool: PgPool) -> ResumeAppService {
    let repo = Arc::new(pg_repository::PgResumeRepository::new(pool));
    service::ResumeService::new(repo)
}
