pub mod pg_repository;
pub mod repository;
mod router;
pub mod service;
pub mod types;

pub use router::routes;

pub type ResumeAppService = service::ResumeService<pg_repository::PgResumeRepository>;

#[must_use]
pub fn wire_resume_service(pool: sqlx::PgPool) -> ResumeAppService {
    let repo = std::sync::Arc::new(pg_repository::PgResumeRepository::new(pool));
    service::ResumeService::new(repo)
}
