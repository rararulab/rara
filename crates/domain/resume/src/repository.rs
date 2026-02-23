use uuid::Uuid;

use crate::types::{ResumeError, ResumeProject};

#[async_trait::async_trait]
pub trait ResumeRepository: Send + Sync {
    async fn create(
        &self,
        id: Uuid,
        name: &str,
        git_url: &str,
        local_path: &str,
    ) -> Result<ResumeProject, ResumeError>;
    async fn get(&self) -> Result<Option<ResumeProject>, ResumeError>;
    async fn get_by_id(&self, id: Uuid) -> Result<Option<ResumeProject>, ResumeError>;
    async fn update_name(&self, id: Uuid, name: &str) -> Result<ResumeProject, ResumeError>;
    async fn update_synced_at(&self, id: Uuid) -> Result<(), ResumeError>;
    async fn delete(&self, id: Uuid) -> Result<(), ResumeError>;
}
