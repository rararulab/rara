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

//! Application-level service for application lifecycle management.
//!
//! [`ApplicationService`] orchestrates status transitions through the
//! [`StateMachine`], records status change history, and delegates
//! persistence to an [`ApplicationRepository`].

use std::sync::Arc;

use chrono::Utc;
use job_domain_core::{ApplicationId, ApplicationStatus};
use tracing::instrument;
use uuid::Uuid;

use crate::{
    error::{ApplicationError, NotFoundSnafu},
    repository::ApplicationRepository,
    state_machine::StateMachine,
    types::{
        Application, ApplicationFilter, ApplicationStatistics, ChangeSource,
        CreateApplicationRequest, StatusChangeRecord, UpdateApplicationRequest,
    },
};

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// High-level service for application CRUD, status transitions, and
/// history tracking.
pub struct ApplicationService {
    repo:          Arc<dyn ApplicationRepository>,
    state_machine: StateMachine,
}

impl ApplicationService {
    /// Create a new service backed by the given repository and the
    /// default state machine rules.
    #[must_use]
    pub fn new(repo: Arc<dyn ApplicationRepository>) -> Self {
        Self {
            repo,
            state_machine: StateMachine::default(),
        }
    }

    /// Create a new service with custom state machine rules.
    #[must_use]
    pub fn with_state_machine(
        repo: Arc<dyn ApplicationRepository>,
        state_machine: StateMachine,
    ) -> Self {
        Self {
            repo,
            state_machine,
        }
    }

    // -- Create -------------------------------------------------------------

    /// Create a new application in `Draft` status.
    #[instrument(skip(self, req))]
    pub async fn create_application(
        &self,
        req: CreateApplicationRequest,
    ) -> Result<Application, ApplicationError> {
        let now = Utc::now();

        let app = Application {
            id:           ApplicationId::new(),
            job_id:       req.job_id,
            resume_id:    req.resume_id,
            channel:      req.channel,
            status:       ApplicationStatus::Draft,
            cover_letter: req.cover_letter,
            notes:        req.notes,
            tags:         req.tags,
            priority:     req.priority,
            trace_id:     None,
            is_deleted:   false,
            submitted_at: None,
            created_at:   now,
            updated_at:   now,
        };

        tracing::info!(
            application_id = %app.id,
            job_id = %app.job_id,
            "Creating new application in Draft status"
        );

        self.repo.save(&app).await
    }

    // -- Status transitions -------------------------------------------------

    /// Transition an application to a new status.
    ///
    /// This method:
    /// 1. Loads the application.
    /// 2. Validates the transition via the state machine.
    /// 3. Updates the application status and timestamps.
    /// 4. Records a [`StatusChangeRecord`].
    /// 5. Persists everything.
    #[instrument(skip(self, note))]
    pub async fn transition_status(
        &self,
        id: ApplicationId,
        new_status: ApplicationStatus,
        source: ChangeSource,
        note: Option<String>,
    ) -> Result<Application, ApplicationError> {
        let mut app = self.get_application_or_err(id).await?;
        let old_status = app.status;

        self.state_machine
            .validate_transition(old_status, new_status)?;

        app.status = new_status;
        app.updated_at = Utc::now();

        // Set submitted_at when first transitioning to Submitted.
        if new_status == ApplicationStatus::Submitted && app.submitted_at.is_none() {
            app.submitted_at = Some(app.updated_at);
        }

        let record = StatusChangeRecord {
            id: Uuid::new_v4(),
            application_id: id,
            from_status: old_status,
            to_status: new_status,
            changed_by: source,
            note,
            created_at: app.updated_at,
        };

        tracing::info!(
            application_id = %id,
            from = %old_status,
            to = %new_status,
            source = %source,
            "Application status transition"
        );

        self.repo.save_status_change(&record).await?;
        self.repo.update(&app).await
    }

    // -- Read ---------------------------------------------------------------

    /// Get a single application by id.
    #[instrument(skip(self))]
    pub async fn get_application(
        &self,
        id: ApplicationId,
    ) -> Result<Application, ApplicationError> {
        self.get_application_or_err(id).await
    }

    /// List applications matching the given filter.
    #[instrument(skip(self))]
    pub async fn list_applications(
        &self,
        filter: &ApplicationFilter,
    ) -> Result<Vec<Application>, ApplicationError> {
        self.repo.find_all(filter).await
    }

    // -- Update -------------------------------------------------------------

    /// Apply a partial update to an application.
    #[instrument(skip(self, req))]
    pub async fn update_application(
        &self,
        id: ApplicationId,
        req: UpdateApplicationRequest,
    ) -> Result<Application, ApplicationError> {
        let mut app = self.get_application_or_err(id).await?;

        if let Some(cover_letter) = req.cover_letter {
            app.cover_letter = cover_letter;
        }
        if let Some(notes) = req.notes {
            app.notes = notes;
        }
        if let Some(tags) = req.tags {
            app.tags = tags;
        }
        if let Some(priority) = req.priority {
            app.priority = priority;
        }
        if let Some(channel) = req.channel {
            app.channel = channel;
        }

        app.updated_at = Utc::now();

        tracing::info!(application_id = %id, "Updating application fields");

        self.repo.update(&app).await
    }

    // -- Delete -------------------------------------------------------------

    /// Soft-delete an application.
    #[instrument(skip(self))]
    pub async fn delete_application(&self, id: ApplicationId) -> Result<(), ApplicationError> {
        // Verify it exists first.
        let _ = self.get_application_or_err(id).await?;
        self.repo.delete(id).await
    }

    // -- History ------------------------------------------------------------

    /// Get the full status change history for an application.
    #[instrument(skip(self))]
    pub async fn get_status_history(
        &self,
        id: ApplicationId,
    ) -> Result<Vec<StatusChangeRecord>, ApplicationError> {
        // Verify the application exists.
        let _ = self.get_application_or_err(id).await?;
        self.repo.get_status_history(id).await
    }

    // -- Statistics ---------------------------------------------------------

    /// Compute aggregate statistics across all non-deleted applications.
    #[instrument(skip(self))]
    pub async fn get_statistics(&self) -> Result<ApplicationStatistics, ApplicationError> {
        let all = self.repo.find_all(&ApplicationFilter::default()).await?;

        let total = all.len();
        let mut counts = std::collections::HashMap::new();
        for app in &all {
            *counts.entry(app.status).or_insert(0usize) += 1;
        }

        let mut by_status: Vec<(ApplicationStatus, usize)> = counts.into_iter().collect();
        by_status.sort_by_key(|(status, _)| format!("{status}"));

        Ok(ApplicationStatistics { total, by_status })
    }

    // -- Helpers ------------------------------------------------------------

    /// Return the application's allowed next statuses.
    #[must_use]
    pub fn allowed_transitions(&self, current: ApplicationStatus) -> Vec<ApplicationStatus> {
        self.state_machine.allowed_transitions(current)
    }

    /// Load an application or return `NotFound`.
    async fn get_application_or_err(
        &self,
        id: ApplicationId,
    ) -> Result<Application, ApplicationError> {
        self.repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| NotFoundSnafu { id }.build())
    }
}

impl std::fmt::Debug for ApplicationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplicationService")
            .field("state_machine", &self.state_machine)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use job_domain_core::{ApplicationId, ApplicationStatus, JobSourceId, ResumeId};

    use super::*;
    use crate::{
        error::ApplicationError,
        repository::ApplicationRepository,
        types::{
            Application, ApplicationChannel, ApplicationFilter, ChangeSource, Priority,
            StatusChangeRecord,
        },
    };

    // -----------------------------------------------------------------------
    // Mock repository
    // -----------------------------------------------------------------------

    #[derive(Debug, Default)]
    struct MockRepo {
        applications: Mutex<Vec<Application>>,
        history:      Mutex<Vec<StatusChangeRecord>>,
    }

    #[async_trait::async_trait]
    impl ApplicationRepository for MockRepo {
        async fn save(&self, app: &Application) -> Result<Application, ApplicationError> {
            let mut apps = self.applications.lock().unwrap();
            apps.push(app.clone());
            Ok(app.clone())
        }

        async fn find_by_id(
            &self,
            id: ApplicationId,
        ) -> Result<Option<Application>, ApplicationError> {
            let apps = self.applications.lock().unwrap();
            Ok(apps.iter().find(|a| a.id == id).cloned())
        }

        async fn find_all(
            &self,
            filter: &ApplicationFilter,
        ) -> Result<Vec<Application>, ApplicationError> {
            let apps = self.applications.lock().unwrap();
            let result = apps
                .iter()
                .filter(|a| filter.status.as_ref().map_or(true, |s| &a.status == s))
                .cloned()
                .collect();
            Ok(result)
        }

        async fn update(&self, app: &Application) -> Result<Application, ApplicationError> {
            let mut apps = self.applications.lock().unwrap();
            if let Some(existing) = apps.iter_mut().find(|a| a.id == app.id) {
                *existing = app.clone();
            }
            Ok(app.clone())
        }

        async fn delete(&self, id: ApplicationId) -> Result<(), ApplicationError> {
            let mut apps = self.applications.lock().unwrap();
            apps.retain(|a| a.id != id);
            Ok(())
        }

        async fn save_status_change(
            &self,
            record: &StatusChangeRecord,
        ) -> Result<(), ApplicationError> {
            let mut history = self.history.lock().unwrap();
            history.push(record.clone());
            Ok(())
        }

        async fn get_status_history(
            &self,
            application_id: ApplicationId,
        ) -> Result<Vec<StatusChangeRecord>, ApplicationError> {
            let history = self.history.lock().unwrap();
            Ok(history
                .iter()
                .filter(|r| r.application_id == application_id)
                .cloned()
                .collect())
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_service() -> (ApplicationService, Arc<MockRepo>) {
        let repo = Arc::new(MockRepo::default());
        let svc = ApplicationService::new(repo.clone());
        (svc, repo)
    }

    fn make_create_request() -> CreateApplicationRequest {
        CreateApplicationRequest {
            job_id:       JobSourceId::new(),
            resume_id:    ResumeId::new(),
            channel:      ApplicationChannel::Direct,
            cover_letter: Some("Hello, I'd like to apply.".to_owned()),
            notes:        None,
            tags:         vec!["rust".to_owned()],
            priority:     Priority::High,
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_application_starts_in_draft() {
        let (svc, _repo) = make_service();
        let req = make_create_request();

        let app = svc.create_application(req).await.unwrap();
        assert_eq!(app.status, ApplicationStatus::Draft);
        assert!(app.submitted_at.is_none());
    }

    #[tokio::test]
    async fn transition_draft_to_submitted() {
        let (svc, _repo) = make_service();
        let req = make_create_request();
        let app = svc.create_application(req).await.unwrap();

        let updated = svc
            .transition_status(
                app.id,
                ApplicationStatus::Submitted,
                ChangeSource::Manual,
                Some("First submission".to_owned()),
            )
            .await
            .unwrap();

        assert_eq!(updated.status, ApplicationStatus::Submitted);
        assert!(updated.submitted_at.is_some());
    }

    #[tokio::test]
    async fn full_happy_path_draft_to_accepted() {
        let (svc, _repo) = make_service();
        let app = svc.create_application(make_create_request()).await.unwrap();
        let id = app.id;

        // Draft -> Submitted -> UnderReview -> Interview -> Offered -> Accepted
        svc.transition_status(id, ApplicationStatus::Submitted, ChangeSource::Manual, None)
            .await
            .unwrap();

        svc.transition_status(
            id,
            ApplicationStatus::UnderReview,
            ChangeSource::EmailParse,
            None,
        )
        .await
        .unwrap();

        svc.transition_status(id, ApplicationStatus::Interview, ChangeSource::System, None)
            .await
            .unwrap();

        svc.transition_status(
            id,
            ApplicationStatus::Offered,
            ChangeSource::EmailParse,
            None,
        )
        .await
        .unwrap();

        let final_app = svc
            .transition_status(
                id,
                ApplicationStatus::Accepted,
                ChangeSource::Manual,
                Some("Accepted the offer!".to_owned()),
            )
            .await
            .unwrap();

        assert_eq!(final_app.status, ApplicationStatus::Accepted);

        // Verify history has 5 entries.
        let history = svc.get_status_history(id).await.unwrap();
        assert_eq!(history.len(), 5);
    }

    #[tokio::test]
    async fn invalid_transition_returns_error() {
        let (svc, _repo) = make_service();
        let app = svc.create_application(make_create_request()).await.unwrap();

        let result = svc
            .transition_status(
                app.id,
                ApplicationStatus::Accepted,
                ChangeSource::Manual,
                None,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ApplicationError::InvalidTransition { .. }
        ));
    }

    #[tokio::test]
    async fn terminal_state_rejects_further_transitions() {
        let (svc, _repo) = make_service();
        let app = svc.create_application(make_create_request()).await.unwrap();
        let id = app.id;

        // Draft -> Withdrawn (terminal).
        svc.transition_status(id, ApplicationStatus::Withdrawn, ChangeSource::Manual, None)
            .await
            .unwrap();

        // Withdrawn -> anything should fail.
        let result = svc
            .transition_status(id, ApplicationStatus::Draft, ChangeSource::Manual, None)
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn update_application_fields() {
        let (svc, _repo) = make_service();
        let app = svc.create_application(make_create_request()).await.unwrap();

        let update_req = UpdateApplicationRequest {
            notes: Some(Some("Updated notes".to_owned())),
            priority: Some(Priority::Critical),
            ..Default::default()
        };

        let updated = svc.update_application(app.id, update_req).await.unwrap();

        assert_eq!(updated.notes.as_deref(), Some("Updated notes"));
        assert_eq!(updated.priority, Priority::Critical);
    }

    #[tokio::test]
    async fn delete_and_get_returns_not_found() {
        let (svc, _repo) = make_service();
        let app = svc.create_application(make_create_request()).await.unwrap();

        svc.delete_application(app.id).await.unwrap();

        let result = svc.get_application(app.id).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ApplicationError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn get_statistics_counts_by_status() {
        let (svc, _repo) = make_service();

        // Create two applications.
        let app1 = svc.create_application(make_create_request()).await.unwrap();
        let _app2 = svc.create_application(make_create_request()).await.unwrap();

        // Move app1 to Submitted.
        svc.transition_status(
            app1.id,
            ApplicationStatus::Submitted,
            ChangeSource::Manual,
            None,
        )
        .await
        .unwrap();

        let stats = svc.get_statistics().await.unwrap();
        assert_eq!(stats.total, 2);
        assert!(
            stats
                .by_status
                .iter()
                .any(|(s, c)| *s == ApplicationStatus::Draft && *c == 1)
        );
        assert!(
            stats
                .by_status
                .iter()
                .any(|(s, c)| *s == ApplicationStatus::Submitted && *c == 1)
        );
    }

    #[tokio::test]
    async fn get_nonexistent_application_returns_not_found() {
        let (svc, _repo) = make_service();
        let result = svc.get_application(ApplicationId::new()).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ApplicationError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn list_applications_with_status_filter() {
        let (svc, _repo) = make_service();

        let app1 = svc.create_application(make_create_request()).await.unwrap();
        let _app2 = svc.create_application(make_create_request()).await.unwrap();

        svc.transition_status(
            app1.id,
            ApplicationStatus::Submitted,
            ChangeSource::Manual,
            None,
        )
        .await
        .unwrap();

        let filter = ApplicationFilter {
            status: Some(ApplicationStatus::Draft),
            ..Default::default()
        };

        let drafts = svc.list_applications(&filter).await.unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].status, ApplicationStatus::Draft);
    }
}
