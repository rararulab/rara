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

pub mod settings;

use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use job_server::{
    dedup_layer::{DedupLayer, DedupLayerConfig},
    grpc::{GrpcServerConfig, hello::HelloService, start_grpc_server},
    http::{RestServerConfig, health_routes, start_rest_server},
};
use smart_default::SmartDefault;
use snafu::{ResultExt, Whatever, whatever};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;
use yunara_store::config::DatabaseConfig;

// ---------------------------------------------------------------------------
// Config types
// ---------------------------------------------------------------------------

/// OpenRouter API configuration.
#[derive(Debug, Clone)]
pub struct OpenRouterConfig {
    pub api_key: String,
    pub model:   String,
}

/// MinIO / S3-compatible object store configuration.
#[derive(Debug, Clone)]
pub struct MinioConfig {
    pub endpoint:   String,
    pub bucket:     String,
    pub access_key: String,
    pub secret_key: String,
    pub region:     String,
}

/// Configuration for the application.
#[derive(Debug, Clone, SmartDefault)]
pub struct AppConfig {
    /// gRPC server configuration
    pub grpc_config:              GrpcServerConfig,
    /// REST server configuration
    pub http_config:              RestServerConfig,
    /// Database configuration
    pub db_config:                DatabaseConfig,
    /// Whether to enable graceful shutdown
    #[default = true]
    pub enable_graceful_shutdown: bool,
    /// OpenRouter configuration (optional)
    pub openrouter:               Option<OpenRouterConfig>,
    /// MinIO / S3 object store configuration (optional)
    pub minio:                    Option<MinioConfig>,
    /// Crawl4AI service URL
    #[default(_code = r#""http://localhost:11235".to_owned()"#)]
    pub crawl4ai_url:             String,
    /// Saved-job GC interval in hours
    #[default = 24]
    pub gc_interval_hours:        u64,
}

impl AppConfig {
    /// Build an `AppConfig` from environment variables and optional config file.
    ///
    /// Uses [`Settings::new()`] which supports layered configuration:
    /// 1. Legacy environment variables (`DATABASE_URL`, `OPENROUTER_API_KEY`, etc.)
    /// 2. `JOB__`-prefixed environment variables
    /// 3. `config.toml` file in the working directory
    /// 4. Code defaults
    pub fn from_env() -> Self {
        match crate::settings::Settings::new() {
            Ok(settings) => settings.into_app_config(),
            Err(e) => {
                tracing::warn!(
                    "Failed to load config via Settings, falling back to defaults: {e}"
                );
                Self::default()
            }
        }
    }

    /// Initialize the database, create all domain services, and return a
    /// ready-to-run [`App`].
    pub async fn open(self) -> Result<App, Whatever> {
        info!("Initializing job application");

        // Initialize database
        let db_store = yunara_store::db::DBStore::new(self.db_config.clone())
            .await
            .whatever_context("Failed to initialize database")?;
        let pool = db_store.pool().clone();

        // Create repository implementations (from domain crates)
        let resume_repo = Arc::new(job_domain_resume::pg_repository::PgResumeRepository::new(
            pool.clone(),
        ));
        let application_repo = Arc::new(
            job_domain_application::pg_repository::PgApplicationRepository::new(pool.clone()),
        );
        let interview_repo = Arc::new(
            job_domain_interview::pg_repository::PgInterviewPlanRepository::new(pool.clone()),
        );
        let scheduler_repo =
            Arc::new(job_domain_scheduler::pg_repository::PgSchedulerRepository::new(pool.clone()));
        let analytics_repo =
            Arc::new(job_domain_analytics::pg_repository::PgAnalyticsRepository::new(pool.clone()));
        let saved_job_repo: Arc<dyn job_domain_saved_job::repository::SavedJobRepository> =
            Arc::new(job_domain_saved_job::pg_repository::PgSavedJobRepository::new(pool.clone()));

        // Runtime settings (DB-backed, env as fallback defaults).
        let fallback_settings = runtime_settings_from_env(self.openrouter.as_ref());
        let settings_service = Arc::new(
            settings::RuntimeSettingsService::load(db_store.kv_store(), fallback_settings)
                .await
                .whatever_context("Failed to initialize runtime settings service")?,
        );
        let initial_settings = settings_service.current();
        let ai_service_handle = Arc::new(RwLock::new(build_ai_service(&initial_settings)));
        if ai_service_handle
            .read()
            .ok()
            .and_then(|g| g.as_ref().cloned())
            .is_some()
        {
            info!("AI service configured from runtime settings");
        } else {
            warn!("AI service not configured yet; set it via POST /api/v1/settings");
        }

        // Scheduler + analytics services
        let scheduler_service = Arc::new(job_domain_scheduler::service::SchedulerService::new(
            scheduler_repo,
        ));
        let analytics_service = Arc::new(job_domain_analytics::service::AnalyticsService::new(
            analytics_repo,
        ));

        // Job repository (for saving parsed JDs)
        let job_repo: Arc<dyn job_domain_job_source::repository::JobRepository> = Arc::new(
            job_domain_job_source::pg_repository::PgJobRepository::new(pool.clone()),
        );

        // Object store (MinIO / S3) — required, for saved-job markdown
        let minio_cfg = match self.minio.as_ref() {
            Some(cfg) => cfg,
            None => {
                whatever!("MinIO is required: set MINIO_ENDPOINT")
            }
        };
        let os_cfg = job_object_store::ObjectStoreConfig::builder()
            .endpoint(minio_cfg.endpoint.clone())
            .bucket(minio_cfg.bucket.clone())
            .access_key(minio_cfg.access_key.clone())
            .secret_key(minio_cfg.secret_key.clone())
            .region(minio_cfg.region.clone())
            .root("/".to_owned())
            .build();
        let object_store = Arc::new(
            job_object_store::ObjectStore::new(&os_cfg)
                .whatever_context("Failed to initialize object store")?,
        );
        info!("Object store (MinIO) configured");

        // Job Source domain — JobSpy driver + discovery service
        let jobspy_driver = job_domain_job_source::jobspy::JobSpyDriver::new()
            .whatever_context("Failed to initialize JobSpy driver")?;
        info!("JobSpy driver initialized");
        let job_source_service = Arc::new(job_domain_job_source::service::JobSourceService::new(
            jobspy_driver,
        ));

        // Domain services
        let resume_service = Arc::new(job_domain_resume::service::ResumeService::new(resume_repo));
        let application_service = Arc::new(
            job_domain_application::service::ApplicationService::new(application_repo),
        );
        let interview_service = Arc::new(job_domain_interview::service::InterviewService::new(
            interview_repo,
            None,
        ));
        let saved_job_service = Arc::new(job_domain_saved_job::service::SavedJobService::new(
            saved_job_repo,
        ));

        // Crawl4AI client (used by CrawlWorker)
        let crawl_client = job_domain_saved_job::crawl4ai::Crawl4AiClient::new(&self.crawl4ai_url);
        info!("Crawl4AI client configured");

        // Build routes closure — captures Arc'd services for on-demand Router
        // construction (axum::Router does not implement Clone).
        let resume_svc = resume_service.clone();
        let app_svc = application_service.clone();
        let interview_svc = interview_service.clone();
        let scheduler_svc = scheduler_service.clone();
        let analytics_svc = analytics_service.clone();
        let saved_job_svc = saved_job_service.clone();
        let job_source_svc = job_source_service.clone();
        let settings_svc = settings_service.clone();

        let ai_handle_for_bot = ai_service_handle.clone();
        let job_repo_for_bot = job_repo.clone();
        let routes_fn: Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync> =
            Box::new(move |router: axum::Router| {
                let router = health_routes(router);
                let bot_router = bot_internal_routes(BotInternalState {
                    ai_service_handle: ai_handle_for_bot.clone(),
                    job_repo:          job_repo_for_bot.clone(),
                });
                router
                    .merge(job_domain_resume::routes::routes(resume_svc.clone()))
                    .merge(job_domain_application::routes::routes(app_svc.clone()))
                    .merge(job_domain_interview::routes::routes(interview_svc.clone()))
                    .merge(job_domain_scheduler::routes::routes(scheduler_svc.clone()))
                    .merge(job_domain_analytics::routes::routes(analytics_svc.clone()))
                    .merge(job_domain_saved_job::routes::routes(saved_job_svc.clone()))
                    .merge(
                        job_domain_job_source::routes::routes(job_source_svc.clone())
                            .layer(DedupLayer::new(DedupLayerConfig::default())),
                    )
                    .merge(settings_routes(
                        settings_svc.clone(),
                        ai_handle_for_bot.clone(),
                    ))
                    .merge(bot_router)
            });

        info!("Application initialized successfully");

        Ok(App {
            config: self,
            running: Arc::new(AtomicBool::new(false)),
            cancellation_token: CancellationToken::new(),
            routes_fn,
            ai_service_handle,
            job_repo,
            saved_job_service,
            object_store,
            crawl_client,
        })
    }
}

fn runtime_settings_from_env(
    openrouter: Option<&OpenRouterConfig>,
) -> job_domain_shared::runtime_settings::RuntimeSettings {
    let mut settings = job_domain_shared::runtime_settings::RuntimeSettings::default();
    if let Some(cfg) = openrouter {
        settings.ai.openrouter_api_key = Some(cfg.api_key.clone());
        settings.ai.model = Some(cfg.model.clone());
    }
    settings.normalize();
    settings
}

fn build_ai_service(
    settings: &job_domain_shared::runtime_settings::RuntimeSettings,
) -> Option<Arc<job_ai::service::AiService>> {
    let api_key = settings.ai.openrouter_api_key.as_deref()?;
    let model = settings
        .ai
        .model
        .clone()
        .unwrap_or_else(|| "openai/gpt-4o".to_owned());
    Some(Arc::new(job_ai::service::AiService::new(
        api_key, model, None,
    )))
}

// ---------------------------------------------------------------------------
// App + AppHandle
// ---------------------------------------------------------------------------

/// Represents the main application with lifecycle management.
pub struct App {
    /// Application configuration
    config:             AppConfig,
    /// Controls if the application should continue running
    running:            Arc<AtomicBool>,
    /// Cancellation token for graceful shutdown
    cancellation_token: CancellationToken,
    /// Closure that builds the axum Router from domain routes
    routes_fn:          Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync>,
    /// Hot-swappable AI service built from current runtime settings.
    ai_service_handle:  Arc<RwLock<Option<Arc<job_ai::service::AiService>>>>,
    /// Job repository for persisting parsed jobs
    job_repo:           Arc<dyn job_domain_job_source::repository::JobRepository>,
    /// Saved job service for workers
    saved_job_service:  Arc<job_domain_saved_job::service::SavedJobService>,
    /// Object store for S3 operations
    object_store:       Arc<job_object_store::ObjectStore>,
    /// Crawl4AI client for CrawlWorker
    crawl_client:       job_domain_saved_job::crawl4ai::Crawl4AiClient,
}

/// Handle for controlling a running application.
#[allow(dead_code)]
pub struct AppHandle {
    /// Sender for triggering shutdown
    shutdown_tx:        Option<oneshot::Sender<()>>,
    /// Application running flag
    running:            Arc<AtomicBool>,
    /// Cancellation token
    cancellation_token: CancellationToken,
}

#[allow(dead_code)]
impl AppHandle {
    /// Gracefully shutdown the application
    pub fn shutdown(&mut self) {
        info!("Initiating graceful shutdown");
        self.running.store(false, Ordering::SeqCst);
        self.cancellation_token.cancel();

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Check if the application is still running
    #[must_use]
    pub fn is_running(&self) -> bool { self.running.load(Ordering::SeqCst) }

    /// Wait for the application to shutdown
    pub async fn wait_for_shutdown(&self) { self.cancellation_token.cancelled().await; }
}

impl App {
    /// Start the application and return a handle for controlling it.
    ///
    /// This only starts servers and background workers — all service
    /// initialization has already been done in [`AppConfig::open()`].
    async fn start(self) -> Result<AppHandle, Whatever> {
        info!("Starting job application");

        // Set running flag
        self.running.store(true, Ordering::SeqCst);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        // Create app handle
        let app_handle = AppHandle {
            shutdown_tx:        Some(shutdown_tx),
            running:            Arc::clone(&self.running),
            cancellation_token: self.cancellation_token.clone(),
        };

        // Start servers
        let mut grpc_handle =
            start_grpc_server(&self.config.grpc_config, &[Arc::new(HelloService)])
                .whatever_context("Failed to start gRPC server")?;

        info!("starting rest server ...");
        let mut http_handle =
            start_rest_server(self.config.http_config.clone(), vec![self.routes_fn])
                .await
                .whatever_context("Failed to start REST server")?;

        // Ensure sockets are actually accepting requests before we continue
        // with worker/bootstrap side effects.
        grpc_handle
            .wait_for_start()
            .await
            .whatever_context("gRPC server failed to report started")?;
        http_handle
            .wait_for_start()
            .await
            .whatever_context("REST server failed to report started")?;

        // Keep a clone for wiring notify trigger after worker_state takes ownership
        let saved_job_svc_for_notify = self.saved_job_service.clone();

        // Shared holder for the analyze worker's notify handle — set after
        // the analyze worker is spawned so the crawl worker can trigger it.
        let analyze_notify = Arc::new(std::sync::RwLock::new(None));

        // Set up background worker manager.
        let worker_state = job_workers::worker_state::AppWorkerState {
            ai_service_handle: self.ai_service_handle,
            job_repo:          self.job_repo,
            saved_job_service: self.saved_job_service,
            object_store:      self.object_store,
            crawl_client:      self.crawl_client,
            analyze_notify:    analyze_notify.clone(),
        };

        // Use an app-owned runtime for workers so shutdown can fully reclaim
        // worker threads instead of relying on global background runtimes.
        let worker_runtime = Arc::new(
            job_common_runtime::RuntimeOptions::builder()
                .thread_name("job-worker".to_owned())
                .enable_io(true)
                .enable_time(true)
                .build()
                .create()
                .whatever_context("Failed to create worker runtime")?,
        );
        let manager_config = job_common_worker::ManagerConfig {
            runtime:          Some(worker_runtime),
            shutdown_timeout: Duration::from_secs(30),
        };
        let mut worker_manager =
            job_common_worker::Manager::with_state_and_config(worker_state, manager_config);

        // Saved job analyze worker (notify trigger, processes Crawled jobs)
        let analyze_handle = worker_manager
            .fallible_worker(job_workers::saved_job_analyze::SavedJobAnalyzeWorker)
            .name("saved-job-analyze")
            .eager()
            .on_notify()
            .spawn();

        // Store the analyze notify handle so CrawlWorker can trigger analysis
        // after crawling completes.
        if let Ok(mut guard) = analyze_notify.write() {
            *guard = Some(analyze_handle.clone());
        }

        // Saved job crawl worker (notify trigger, processes PendingCrawl jobs)
        let crawl_handle = worker_manager
            .fallible_worker(job_workers::saved_job_crawl::SavedJobCrawlWorker)
            .name("saved-job-crawl")
            .eager()
            .on_notify()
            .spawn();
        saved_job_svc_for_notify.set_notify_trigger(crawl_handle.clone());

        // Saved job GC worker (periodic, default every 24 hours)
        let gc_interval_secs = self.config.gc_interval_hours * 3600;
        let _gc_handle = worker_manager
            .fallible_worker(job_workers::saved_job_gc::SavedJobGcWorker::new(
                job_workers::saved_job_gc::GcConfig::default(),
            ))
            .name("saved-job-gc")
            .interval(std::time::Duration::from_secs(gc_interval_secs))
            .spawn();

        info!("Application started successfully");

        // Spawn the main application loop
        let running = Arc::clone(&self.running);
        let cancellation_token = self.cancellation_token.clone();
        let enable_graceful_shutdown = self.config.enable_graceful_shutdown;

        tokio::spawn(async move {
            if enable_graceful_shutdown {
                shutdown_signal(shutdown_rx).await;
            } else {
                // Just wait for explicit shutdown if graceful shutdown is disabled
                let _ = shutdown_rx.await;
            }

            running.store(false, Ordering::SeqCst);
            cancellation_token.cancel();

            // Shutdown background workers. Do not block forever here:
            // if one worker gets stuck, we must still tear down servers.
            info!("Shutting down background workers");
            if tokio::time::timeout(
                std::time::Duration::from_secs(10),
                worker_manager.shutdown(),
            )
            .await
            .is_err()
            {
                error!("Worker manager shutdown timed out; continuing shutdown");
            }

            // Shutdown servers and standalone services
            info!("Shutting down servers");
            grpc_handle.shutdown();
            http_handle.shutdown();

            info!("Application shutdown complete");
        });

        Ok(app_handle)
    }

    /// Run the application blocking until it's shut down.
    pub async fn run(self) -> Result<(), Whatever> {
        let handle = self.start().await?;
        handle.wait_for_shutdown().await;
        Ok(())
    }
}

async fn shutdown_signal(shutdown_rx: oneshot::Receiver<()>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => { info!("Received Ctrl+C signal"); },
        () = terminate => { info!("Received terminate signal"); },
        _ = shutdown_rx => { info!("Received shutdown signal"); },
    }
}

#[derive(Clone)]
struct BotInternalState {
    ai_service_handle: Arc<RwLock<Option<Arc<job_ai::service::AiService>>>>,
    job_repo:          Arc<dyn job_domain_job_source::repository::JobRepository>,
}

#[derive(Debug, serde::Deserialize)]
struct BotJdParseRequest {
    text: String,
}

#[derive(Debug, serde::Deserialize)]
struct ParsedJob {
    title:           String,
    company:         String,
    location:        Option<String>,
    description:     Option<String>,
    url:             Option<String>,
    salary_min:      Option<i32>,
    salary_max:      Option<i32>,
    salary_currency: Option<String>,
    tags:            Option<Vec<String>>,
}

#[derive(Debug, serde::Serialize)]
struct BotJdParseResponse {
    id:      Uuid,
    title:   String,
    company: String,
}

fn bot_internal_routes(state: BotInternalState) -> Router {
    Router::new()
        .route("/api/v1/internal/bot/jd-parse", post(parse_jd_from_bot))
        .with_state(state)
}

#[derive(Clone)]
struct SettingsRouteState {
    settings_service:  Arc<settings::RuntimeSettingsService>,
    ai_service_handle: Arc<RwLock<Option<Arc<job_ai::service::AiService>>>>,
}

fn settings_routes(
    settings_service: Arc<settings::RuntimeSettingsService>,
    ai_service_handle: Arc<RwLock<Option<Arc<job_ai::service::AiService>>>>,
) -> Router {
    Router::new()
        .route("/api/v1/settings", axum::routing::get(get_settings))
        .route("/api/v1/settings", post(update_settings))
        .with_state(SettingsRouteState {
            settings_service,
            ai_service_handle,
        })
}

async fn get_settings(
    State(state): State<SettingsRouteState>,
) -> Result<Json<settings::RuntimeSettingsView>, (StatusCode, String)> {
    let current = state.settings_service.current();
    Ok(Json(settings::to_view(&current)))
}

async fn update_settings(
    State(state): State<SettingsRouteState>,
    Json(patch): Json<job_domain_shared::runtime_settings::RuntimeSettingsPatch>,
) -> Result<Json<settings::RuntimeSettingsView>, (StatusCode, String)> {
    let updated = state.settings_service.update(patch).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to update runtime settings: {e}"),
        )
    })?;

    let next_ai = build_ai_service(&updated);
    if let Ok(mut guard) = state.ai_service_handle.write() {
        *guard = next_ai;
    } else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to lock ai runtime handle".to_owned(),
        ));
    }

    Ok(Json(settings::to_view(&updated)))
}

async fn parse_jd_from_bot(
    State(state): State<BotInternalState>,
    Json(req): Json<BotJdParseRequest>,
) -> Result<(StatusCode, Json<BotJdParseResponse>), (StatusCode, String)> {
    if req.text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "text must not be empty".to_owned()));
    }

    let ai_service = state
        .ai_service_handle
        .read()
        .ok()
        .and_then(|g| g.as_ref().cloned())
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "ai service not configured; set OPENROUTER key/model via /api/v1/settings".to_owned(),
        ))?;

    let json_str = ai_service.jd_parser().parse(&req.text).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to parse jd via ai service: {e}"),
        )
    })?;

    let parsed: ParsedJob = serde_json::from_str(&json_str).map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to deserialize ai response: {e}"),
        )
    })?;

    let job = job_domain_job_source::types::NormalizedJob {
        id:              Uuid::new_v4(),
        source_job_id:   Uuid::new_v4().to_string(),
        source_name:     "telegram".to_owned(),
        title:           parsed.title,
        company:         parsed.company,
        location:        parsed.location,
        description:     parsed.description,
        url:             parsed.url,
        salary_min:      parsed.salary_min,
        salary_max:      parsed.salary_max,
        salary_currency: parsed.salary_currency,
        tags:            parsed.tags.unwrap_or_default(),
        raw_data:        serde_json::to_value(&req.text).ok(),
        posted_at:       None,
    };

    let saved = state.job_repo.save(&job).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to save parsed jd job: {e}"),
        )
    })?;

    Ok((
        StatusCode::OK,
        Json(BotJdParseResponse {
            id:      saved.id,
            title:   saved.title,
            company: saved.company,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = AppConfig::default();
        assert!(config.enable_graceful_shutdown);
        assert!(config.openrouter.is_none());
    }

    #[tokio::test]
    async fn test_app_handle_shutdown() {
        // open() requires a database, so bail if it fails
        let config = AppConfig::default();
        let Ok(app) = config.open().await else {
            return;
        };

        let result = app.start().await;

        // If it fails to start, that's expected in test environment
        if result.is_err() {
            return;
        }

        let mut handle = result.unwrap();
        assert!(handle.is_running());

        // Test shutdown
        handle.shutdown();

        // Wait a bit for shutdown to complete
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert!(!handle.is_running());
    }
}
