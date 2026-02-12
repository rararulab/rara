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

//! Unified job service — discovery, saved-job tracking, and AI-powered JD
//! parsing.

use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, RwLock},
};

use jiff::Timestamp;
use job_common_worker::{Notifiable, NotifyHandle};
use tracing::{instrument, warn};
use uuid::Uuid;

use crate::{
    dedup::{self, FuzzyKey, SourceKey},
    error::{SavedJobError, SourceError},
    jobspy::JobSpyDriver,
    repository::{JobRepository, SavedJobRepository},
    types::{
        DiscoveryCriteria, NormalizedJob, ParsedJob, PipelineEvent, PipelineEventKind,
        PipelineStage, RawJob, SavedJob, SavedJobStatus,
    },
};

// ===========================================================================
// JobService
// ===========================================================================

/// Unified service for job discovery, saved-job management, and JD parsing.
#[derive(Clone)]
pub struct JobService {
    driver:         Arc<JobSpyDriver>,
    saved_job_repo: Arc<dyn SavedJobRepository>,
    job_repo:       Arc<dyn JobRepository>,
    ai_service:     job_ai::service::AiService,
    notify_trigger: Arc<RwLock<Option<NotifyHandle>>>,
}

impl JobService {
    /// Create a new unified job service.
    pub fn new(
        driver: JobSpyDriver,
        saved_job_repo: Arc<dyn SavedJobRepository>,
        job_repo: Arc<dyn JobRepository>,
        ai_service: job_ai::service::AiService,
    ) -> Self {
        Self {
            driver: Arc::new(driver),
            saved_job_repo,
            job_repo,
            ai_service,
            notify_trigger: Arc::new(RwLock::new(None)),
        }
    }

    // -- Accessors ----------------------------------------------------------

    /// Access the AI service (for workers that need `jd_analyzer` etc.).
    pub fn ai_service(&self) -> &job_ai::service::AiService { &self.ai_service }

    /// Access the job repository directly.
    pub fn job_repo(&self) -> &Arc<dyn JobRepository> { &self.job_repo }

    // -- Worker coordination ------------------------------------------------

    /// Registers the runtime notify handle used to trigger immediate
    /// pipeline processing when new jobs are saved.
    pub fn set_notify_trigger(&self, handle: NotifyHandle) {
        if let Ok(mut guard) = self.notify_trigger.write() {
            *guard = Some(handle);
        } else {
            warn!("failed to acquire saved-job notify trigger write lock");
        }
    }

    fn trigger_pipeline(&self) {
        if let Ok(guard) = self.notify_trigger.read() {
            if let Some(handle) = guard.as_ref() {
                handle.notify();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Discover jobs matching the criteria, returning deduplicated results.
    ///
    /// `existing_source_keys` and `existing_fuzzy_keys` represent jobs that
    /// already exist in the database so they can be excluded during
    /// deduplication.
    pub fn discover(
        &self,
        criteria: &DiscoveryCriteria,
        existing_source_keys: &HashSet<SourceKey>,
        existing_fuzzy_keys: &HashSet<FuzzyKey>,
    ) -> DiscoveryResult {
        let raw_jobs = match self.driver.fetch_jobs(criteria) {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::error!(error = %e, "JobSpy driver failed during discovery");
                return DiscoveryResult {
                    jobs:  vec![],
                    error: Some(e),
                };
            }
        };

        tracing::info!(count = raw_jobs.len(), "JobSpy returned raw jobs");
        log_description_coverage_by_source(&raw_jobs);

        // Normalize raw -> NormalizedJob via TryFrom.
        let mut normalized = Vec::with_capacity(raw_jobs.len());
        for raw in raw_jobs {
            match NormalizedJob::try_from(raw) {
                Ok(job) => normalized.push(job),
                Err(e) => {
                    tracing::warn!(error = %e, "Skipping job that failed normalization");
                }
            }
        }
        tracing::info!(
            normalized_count = normalized.len(),
            "normalization complete"
        );

        let mut deduped = dedup::deduplicate(normalized, existing_source_keys, existing_fuzzy_keys);

        // Sort by posted_at descending (newest first, None last).
        deduped.sort_by(|a, b| b.posted_at.cmp(&a.posted_at));

        tracing::info!(
            deduped_count = deduped.len(),
            "deduplication and sorting complete"
        );

        DiscoveryResult {
            jobs:  deduped,
            error: None,
        }
    }

    // -----------------------------------------------------------------------
    // Saved job CRUD
    // -----------------------------------------------------------------------

    /// Save a new job by URL.
    #[instrument(skip(self))]
    pub async fn create(&self, url: &str) -> Result<SavedJob, SavedJobError> {
        let url = url.trim();
        if url.is_empty() {
            return Err(SavedJobError::ValidationError {
                message: "url must not be empty".to_owned(),
            });
        }
        let job = self.saved_job_repo.create(url).await?;
        let _ = self
            .log_event(
                job.id,
                PipelineStage::Crawl,
                PipelineEventKind::Info,
                "job saved, pending crawl",
                None,
            )
            .await;
        self.trigger_pipeline();
        Ok(job)
    }

    /// Get a saved job by id.
    #[instrument(skip(self))]
    pub async fn get(&self, id: Uuid) -> Result<Option<SavedJob>, SavedJobError> {
        self.saved_job_repo.get_by_id(id).await
    }

    /// List saved jobs, optionally filtered by status.
    #[instrument(skip(self))]
    pub async fn list(
        &self,
        status: Option<SavedJobStatus>,
    ) -> Result<Vec<SavedJob>, SavedJobError> {
        self.saved_job_repo.list(status).await
    }

    /// Delete a saved job.
    #[instrument(skip(self))]
    pub async fn delete(&self, id: Uuid) -> Result<(), SavedJobError> {
        self.saved_job_repo.delete(id).await
    }

    /// Update the pipeline status (and optionally record an error).
    #[instrument(skip(self))]
    pub async fn update_status(
        &self,
        id: Uuid,
        status: SavedJobStatus,
        error_message: Option<String>,
    ) -> Result<(), SavedJobError> {
        self.saved_job_repo
            .update_status(id, status, error_message)
            .await
    }

    /// Store the crawl result.
    #[instrument(skip(self, preview))]
    pub async fn update_crawl_result(
        &self,
        id: Uuid,
        s3_key: &str,
        preview: &str,
    ) -> Result<(), SavedJobError> {
        self.saved_job_repo
            .update_crawl_result(id, s3_key, preview)
            .await
    }

    /// Store the analysis result.
    #[instrument(skip(self, result))]
    pub async fn update_analysis(
        &self,
        id: Uuid,
        result: serde_json::Value,
        score: f32,
    ) -> Result<(), SavedJobError> {
        self.saved_job_repo.update_analysis(id, result, score).await
    }

    /// Retry a failed or expired saved job by resetting its status to
    /// `PendingCrawl` and clearing the error.
    #[instrument(skip(self))]
    pub async fn retry(&self, id: Uuid) -> Result<(), SavedJobError> {
        self.saved_job_repo
            .update_status(id, SavedJobStatus::PendingCrawl, None)
            .await?;
        let _ = self
            .log_event(
                id,
                PipelineStage::Crawl,
                PipelineEventKind::Info,
                "retry initiated",
                None,
            )
            .await;
        self.trigger_pipeline();
        Ok(())
    }

    /// List saved jobs older than the given timestamp that are not in a
    /// terminal status (Failed or Expired).
    #[instrument(skip(self))]
    pub async fn list_stale(&self, older_than: Timestamp) -> Result<Vec<SavedJob>, SavedJobError> {
        self.saved_job_repo.list_stale(older_than).await
    }

    /// List saved jobs matching the given statuses that have S3 keys set.
    #[instrument(skip(self))]
    pub async fn list_with_s3_keys_by_status(
        &self,
        statuses: &[SavedJobStatus],
    ) -> Result<Vec<SavedJob>, SavedJobError> {
        self.saved_job_repo
            .list_with_s3_keys_by_status(statuses)
            .await
    }

    /// Clear the S3 key for a saved job after object cleanup.
    #[instrument(skip(self))]
    pub async fn clear_s3_key(&self, id: Uuid) -> Result<(), SavedJobError> {
        self.saved_job_repo.clear_s3_key(id).await
    }

    /// Update the title and/or company extracted from AI analysis.
    #[instrument(skip(self))]
    pub async fn update_title_company(
        &self,
        id: Uuid,
        title: Option<String>,
        company: Option<String>,
    ) -> Result<(), SavedJobError> {
        self.saved_job_repo
            .update_title_company(id, title, company)
            .await
    }

    /// Record a pipeline event for a saved job.
    #[instrument(skip(self, metadata))]
    pub async fn log_event(
        &self,
        saved_job_id: Uuid,
        stage: PipelineStage,
        event_kind: PipelineEventKind,
        message: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<PipelineEvent, SavedJobError> {
        self.saved_job_repo
            .create_event(saved_job_id, stage, event_kind, message, metadata)
            .await
    }

    /// List all pipeline events for a saved job.
    #[instrument(skip(self))]
    pub async fn list_events(
        &self,
        saved_job_id: Uuid,
    ) -> Result<Vec<PipelineEvent>, SavedJobError> {
        self.saved_job_repo.list_events(saved_job_id).await
    }

    // -----------------------------------------------------------------------
    // AI-powered JD parsing
    // -----------------------------------------------------------------------

    /// Parse a job description text via AI and save as a [`NormalizedJob`].
    pub async fn parse_jd(&self, text: &str) -> Result<NormalizedJob, SourceError> {
        let agent = self
            .ai_service
            .jd_parser()
            .map_err(|e| SourceError::NonRetryable {
                source_name: "ai".to_owned(),
                message:     format!("ai service not available: {e}"),
            })?;

        let json_str = agent
            .parse(text)
            .await
            .map_err(|e| SourceError::NonRetryable {
                source_name: "ai".to_owned(),
                message:     format!("failed to parse jd: {e}"),
            })?;

        let parsed: ParsedJob =
            serde_json::from_str(&json_str).map_err(|e| SourceError::NonRetryable {
                source_name: "ai".to_owned(),
                message:     format!("failed to deserialize ai response: {e}"),
            })?;

        let job = NormalizedJob::from_parsed(parsed, text);
        self.job_repo.save(&job).await
    }
}

// ===========================================================================
// DiscoveryResult
// ===========================================================================

/// The outcome of a discovery run.
#[derive(Debug)]
pub struct DiscoveryResult {
    /// Successfully normalized and deduplicated jobs.
    pub jobs:  Vec<NormalizedJob>,
    /// If the driver encountered an unrecoverable error, it is captured here.
    pub error: Option<SourceError>,
}

// ===========================================================================
// Private helpers (description coverage logging)
// ===========================================================================

fn log_description_coverage_by_source(raw_jobs: &[RawJob]) {
    for (source, stat) in description_coverage_by_source(raw_jobs) {
        tracing::info!(
            source,
            total = stat.total,
            with_description = stat.with_description,
            without_description = stat.without_description,
            coverage_pct = stat.coverage_pct(),
            "job description coverage by source"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DescriptionCoverage {
    total:               u32,
    with_description:    u32,
    without_description: u32,
}

impl DescriptionCoverage {
    fn coverage_pct(self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.with_description as f64 * 100.0 / self.total as f64
        }
    }
}

fn description_coverage_by_source(raw_jobs: &[RawJob]) -> BTreeMap<String, DescriptionCoverage> {
    let mut by_source: BTreeMap<String, DescriptionCoverage> = BTreeMap::new();
    for job in raw_jobs {
        let stat = by_source
            .entry(job.source_name.clone())
            .or_insert(DescriptionCoverage {
                total:               0,
                with_description:    0,
                without_description: 0,
            });
        stat.total += 1;
        if has_description(job.description.as_deref()) {
            stat.with_description += 1;
        } else {
            stat.without_description += 1;
        }
    }
    by_source
}

fn has_description(description: Option<&str>) -> bool {
    description.is_some_and(|text| !text.trim().is_empty())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_raw(source_name: &str, description: Option<&str>) -> RawJob {
        RawJob {
            source_job_id:   format!("{source_name}-id"),
            source_name:     source_name.to_owned(),
            title:           Some("title".to_owned()),
            company:         Some("company".to_owned()),
            location:        None,
            description:     description.map(ToOwned::to_owned),
            url:             None,
            salary_min:      None,
            salary_max:      None,
            salary_currency: None,
            tags:            vec![],
            raw_data:        None,
            posted_at:       None,
        }
    }

    #[test]
    fn description_coverage_by_source_counts_per_source() {
        let raw_jobs = vec![
            make_raw("indeed", Some("job details")),
            make_raw("indeed", Some("  ")),
            make_raw("linkedin", None),
            make_raw("linkedin", Some("desc")),
        ];

        let stats = description_coverage_by_source(&raw_jobs);
        let indeed = stats.get("indeed").unwrap();
        let linkedin = stats.get("linkedin").unwrap();

        assert_eq!(indeed.total, 2);
        assert_eq!(indeed.with_description, 1);
        assert_eq!(indeed.without_description, 1);
        assert_eq!(indeed.coverage_pct(), 50.0);

        assert_eq!(linkedin.total, 2);
        assert_eq!(linkedin.with_description, 1);
        assert_eq!(linkedin.without_description, 1);
        assert_eq!(linkedin.coverage_pct(), 50.0);
    }

    #[test]
    fn has_description_treats_blank_text_as_missing() {
        assert!(has_description(Some("x")));
        assert!(!has_description(Some("   ")));
        assert!(!has_description(None));
    }
}
