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

//! Unified application state shared by workers and routes.

use std::sync::{Arc, RwLock};

use job_common_worker::NotifyHandle;
use opendal::Operator;
use snafu::{ResultExt, Whatever};
use tracing::{info, warn};
use yunara_store::db::DBStore;

/// Shared application state used by workers and HTTP routes.
#[derive(Clone)]
pub struct AppState {
    // -- AI --
    pub ai_service: job_ai::service::AiService,

    // -- domain services --
    pub resume_service:      job_domain_resume::ResumeAppService,
    pub application_service: job_domain_application::service::ApplicationService,
    pub interview_service:   job_domain_interview::service::InterviewService,
    pub scheduler_service:   job_domain_scheduler::service::SchedulerService,
    pub analytics_service:   job_domain_analytics::service::AnalyticsService,
    pub saved_job_service:   job_domain_job_tracker::service::SavedJobService,
    pub job_source_service:  job_domain_job_discovery::service::JobSourceService,
    pub job_repo:            Arc<dyn job_domain_job_discovery::repository::JobRepository>,

    // -- shared --
    pub settings_svc:  job_domain_shared::settings::SettingsSvc,
    pub notify_client: job_domain_shared::notify::client::NotifyClient,

    // -- infra --
    pub object_store: Operator,
    pub crawl_client: job_domain_job_tracker::crawl4ai::Crawl4AiClient,

    // -- worker coordination --
    pub analyze_notify: Arc<RwLock<Option<NotifyHandle>>>,
}

impl AppState {
    /// Initialize all domain services and build the shared application state.
    pub async fn init(
        db_store: &DBStore,
        object_store: Operator,
        notify_client: job_domain_shared::notify::client::NotifyClient,
        crawl4ai_url: &str,
    ) -> Result<Self, Whatever> {
        let pool = db_store.pool().clone();

        // -- runtime settings ------------------------------------------------

        let settings_svc =
            job_domain_shared::settings::SettingsSvc::load(db_store.kv_store())
                .await
                .whatever_context("Failed to initialize runtime settings")?;
        info!("Runtime settings service loaded");

        // -- AI service ------------------------------------------------------

        let ai_service = job_ai::service::AiService::new(settings_svc.clone());
        if settings_svc.current().ai.openrouter_api_key.is_some() {
            info!("AI service configured from runtime settings");
        } else {
            warn!("AI service not configured yet; set it via POST /api/v1/settings");
        }

        // -- domain services -------------------------------------------------

        let resume_service = job_domain_resume::wire_resume_service(pool.clone());
        let application_service = job_domain_application::wire(pool.clone());
        let interview_service =
            job_domain_interview::wire_interview_service(pool.clone());
        let scheduler_service =
            job_domain_scheduler::wire_scheduler_service(pool.clone());
        let analytics_service =
            job_domain_analytics::wire_analytics_service(pool.clone());
        let saved_job_service =
            job_domain_job_tracker::wire_saved_job_service(pool.clone());
        let job_repo = job_domain_job_discovery::wire_job_repository(pool);

        let job_source_service = job_domain_job_discovery::wire_job_source_service()
            .whatever_context("Failed to initialize JobSpy driver")?;
        info!("JobSpy driver initialized");

        // -- infra clients ---------------------------------------------------

        let crawl_client =
            job_domain_job_tracker::crawl4ai::Crawl4AiClient::new(crawl4ai_url);
        info!("Crawl4AI client configured");

        Ok(Self {
            ai_service,
            resume_service,
            application_service,
            interview_service,
            scheduler_service,
            analytics_service,
            saved_job_service,
            job_source_service,
            job_repo,
            settings_svc,
            notify_client,
            object_store,
            crawl_client,
            analyze_notify: Arc::new(RwLock::new(None)),
        })
    }

    /// Build an [`axum::Router`] with all domain API routes.
    pub fn routes(&self) -> axum::Router {
        use job_server::dedup_layer::{DedupLayer, DedupLayerConfig};

        axum::Router::new()
            .merge(job_domain_resume::routes::routes(
                self.resume_service.clone(),
            ))
            .merge(job_domain_application::routes::routes(
                self.application_service.clone(),
            ))
            .merge(job_domain_interview::routes::routes(
                self.interview_service.clone(),
            ))
            .merge(job_domain_scheduler::routes::routes(
                self.scheduler_service.clone(),
            ))
            .merge(job_domain_analytics::routes::routes(
                self.analytics_service.clone(),
            ))
            .merge(job_domain_job_tracker::routes::routes(
                self.saved_job_service.clone(),
            ))
            .merge(
                job_domain_job_discovery::routes::routes(
                    self.job_source_service.clone(),
                )
                .layer(DedupLayer::new(DedupLayerConfig::default())),
            )
            .merge(job_domain_shared::settings::router::routes(
                self.settings_svc.clone(),
            ))
            .merge(job_domain_shared::notify::routes::routes(
                self.notify_client.clone(),
            ))
            .merge(job_domain_job_tracker::bot_internal_routes::routes(
                self.ai_service.clone(),
                self.job_repo.clone(),
            ))
    }
}
