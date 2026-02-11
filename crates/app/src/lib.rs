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

use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use job_server::{
    dedup_layer::{DedupLayer, DedupLayerConfig},
    grpc::{GrpcServerConfig, hello::HelloService, start_grpc_server},
    http::{RestServerConfig, health_routes, start_rest_server},
};
use serde::Deserialize;
use snafu::{ResultExt, Whatever};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use yunara_store::config::DatabaseConfig;

// ---------------------------------------------------------------------------
// Static config types (immutable after startup)
// ---------------------------------------------------------------------------

/// MinIO / S3-compatible object store configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MinioConfig {
    pub endpoint:   String,
    pub bucket:     String,
    pub access_key: String,
    pub secret_key: String,
    pub region:     String,
}

impl Default for MinioConfig {
    fn default() -> Self {
        Self {
            endpoint:   "http://localhost:9000".to_owned(),
            bucket:     "job-markdown".to_owned(),
            access_key: "minioadmin".to_owned(),
            secret_key: "minioadmin".to_owned(),
            region:     "us-east-1".to_owned(),
        }
    }
}

/// Crawl4AI service configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Crawl4AiConfig {
    pub url: String,
}

impl Default for Crawl4AiConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:11235".to_owned(),
        }
    }
}

/// Static application configuration — immutable after startup.
///
/// Deserializable from `config.toml` + environment variables via the `config`
/// crate. For runtime-changeable values (OpenRouter key, Telegram token, …) see
/// [`job_domain_shared::runtime_settings_service::RuntimeSettingsService`].
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Database connection pool.
    pub database:               DatabaseConfig,
    /// HTTP server bind / limits.
    pub http:                   RestServerConfig,
    /// gRPC server bind / limits.
    pub grpc:                   GrpcServerConfig,
    /// MinIO / S3-compatible object store.
    pub minio:                  MinioConfig,
    /// Crawl4AI service.
    pub crawl4ai:               Crawl4AiConfig,
    /// Saved-job GC interval in hours.
    pub gc_interval_hours:      u64,
    /// Main service HTTP base URL (for telegram bot → main service calls).
    pub main_service_http_base: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            database:               DatabaseConfig::default(),
            http:                   RestServerConfig::default(),
            grpc:                   GrpcServerConfig::default(),
            minio:                  MinioConfig::default(),
            crawl4ai:               Crawl4AiConfig::default(),
            gc_interval_hours:      24,
            main_service_http_base: "http://127.0.0.1:3000".to_owned(),
        }
    }
}

impl AppConfig {
    /// Load config from config file + environment variables.
    ///
    /// Source priority (highest first):
    /// 1. Legacy environment variables (`DATABASE_URL`, `MINIO_ENDPOINT`, etc.)
    /// 2. `JOB__`-prefixed environment variables
    /// 3. `config.toml` file in the working directory
    /// 4. Code defaults
    pub fn new() -> Result<Self, config::ConfigError> {
        let builder = config::Config::builder()
            .add_source(config::File::with_name("config").required(false))
            .add_source(
                config::Environment::with_prefix("JOB")
                    .separator("__")
                    .try_parsing(true),
            )
            .set_override_option("database.database_url", std::env::var("DATABASE_URL").ok())?
            .set_override_option("minio.endpoint", std::env::var("MINIO_ENDPOINT").ok())?
            .set_override_option("minio.bucket", std::env::var("MINIO_BUCKET").ok())?
            .set_override_option("minio.access_key", std::env::var("MINIO_ACCESS_KEY").ok())?
            .set_override_option("minio.secret_key", std::env::var("MINIO_SECRET_KEY").ok())?
            .set_override_option("minio.region", std::env::var("MINIO_REGION").ok())?
            .set_override_option("crawl4ai.url", std::env::var("CRAWL4AI_URL").ok())?
            .set_override_option("gc_interval_hours", std::env::var("GC_INTERVAL_HOURS").ok())?
            .set_override_option(
                "main_service_http_base",
                std::env::var("MAIN_SERVICE_HTTP_BASE").ok(),
            )?;

        let cfg = builder.build()?;
        cfg.try_deserialize()
    }

    /// Initialize the database, create all domain services, and return a
    /// ready-to-run [`App`].
    pub async fn open(self) -> Result<App, Whatever> {
        info!("Initializing job application");

        // Initialize database
        let db_store = yunara_store::db::DBStore::new(self.database.clone())
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
        let saved_job_repo: Arc<dyn job_domain_job_tracker::repository::SavedJobRepository> =
            Arc::new(job_domain_job_tracker::pg_repository::PgSavedJobRepository::new(pool.clone()));

        // Runtime settings (DB-backed, env as fallback defaults).
        let fallback_settings = runtime_settings_from_env();
        let settings_service = Arc::new(
            job_domain_shared::runtime_settings_service::RuntimeSettingsService::load(
                db_store.kv_store(),
                fallback_settings,
            )
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
        let job_repo: Arc<dyn job_domain_job_discovery::repository::JobRepository> = Arc::new(
            job_domain_job_discovery::pg_repository::PgJobRepository::new(pool.clone()),
        );

        // Object store (MinIO / S3) — required, for saved-job markdown
        let os_cfg = job_object_store::ObjectStoreConfig::builder()
            .endpoint(self.minio.endpoint.clone())
            .bucket(self.minio.bucket.clone())
            .access_key(self.minio.access_key.clone())
            .secret_key(self.minio.secret_key.clone())
            .region(self.minio.region.clone())
            .root("/".to_owned())
            .build();
        let object_store = Arc::new(
            job_object_store::ObjectStore::new(&os_cfg)
                .whatever_context("Failed to initialize object store")?,
        );
        info!("Object store (MinIO) configured");

        // Job Source domain — JobSpy driver + discovery service
        let jobspy_driver = job_domain_job_discovery::jobspy::JobSpyDriver::new()
            .whatever_context("Failed to initialize JobSpy driver")?;
        info!("JobSpy driver initialized");
        let job_source_service = Arc::new(job_domain_job_discovery::service::JobSourceService::new(
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
        let saved_job_service = Arc::new(job_domain_job_tracker::service::SavedJobService::new(
            saved_job_repo,
        ));

        // Crawl4AI client (used by CrawlWorker)
        let crawl_client = job_domain_job_tracker::crawl4ai::Crawl4AiClient::new(&self.crawl4ai.url);
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
                router
                    .merge(job_domain_resume::routes::routes(resume_svc.clone()))
                    .merge(job_domain_application::routes::routes(app_svc.clone()))
                    .merge(job_domain_interview::routes::routes(interview_svc.clone()))
                    .merge(job_domain_scheduler::routes::routes(scheduler_svc.clone()))
                    .merge(job_domain_analytics::routes::routes(analytics_svc.clone()))
                    .merge(job_domain_job_tracker::routes::routes(saved_job_svc.clone()))
                    .merge(
                        job_domain_job_discovery::routes::routes(job_source_svc.clone())
                            .layer(DedupLayer::new(DedupLayerConfig::default())),
                    )
                    .merge(job_domain_shared::runtime_settings_routes::routes(
                        settings_svc.clone(),
                        Some(Arc::new({
                            let ai_handle = ai_handle_for_bot.clone();
                            move |updated| {
                                let next_ai = build_ai_service(updated);
                                let mut guard = ai_handle
                                    .write()
                                    .map_err(|_| "failed to lock ai runtime handle".to_owned())?;
                                *guard = next_ai;
                                Ok(())
                            }
                        })),
                    ))
                    .merge(job_domain_job_tracker::bot_internal_routes::routes(
                        ai_handle_for_bot.clone(),
                        job_repo_for_bot.clone(),
                    ))
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

/// Build fallback [`RuntimeSettings`] from environment variables.
///
/// These are only used as defaults when the KV store has no persisted settings.
fn runtime_settings_from_env() -> job_domain_shared::runtime_settings::RuntimeSettings {
    let mut settings = job_domain_shared::runtime_settings::RuntimeSettings::default();
    settings.ai.openrouter_api_key = std::env::var("OPENROUTER_API_KEY").ok();
    settings.ai.model = std::env::var("OPENROUTER_MODEL").ok();
    settings.telegram.bot_token = std::env::var("TELEGRAM_BOT_TOKEN").ok();
    settings.telegram.chat_id = std::env::var("TELEGRAM_CHAT_ID")
        .ok()
        .and_then(|s| s.parse().ok());
    settings.normalize();
    settings
}

pub(crate) fn build_ai_service(
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
    job_repo:           Arc<dyn job_domain_job_discovery::repository::JobRepository>,
    /// Saved job service for workers
    saved_job_service:  Arc<job_domain_job_tracker::service::SavedJobService>,
    /// Object store for S3 operations
    object_store:       Arc<job_object_store::ObjectStore>,
    /// Crawl4AI client for CrawlWorker
    crawl_client:       job_domain_job_tracker::crawl4ai::Crawl4AiClient,
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
        let mut grpc_handle = start_grpc_server(&self.config.grpc, &[Arc::new(HelloService)])
            .whatever_context("Failed to start gRPC server")?;

        info!("starting rest server ...");
        let mut http_handle = start_rest_server(self.config.http.clone(), vec![self.routes_fn])
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
        let enable_graceful_shutdown = true;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = AppConfig::default();
        assert_eq!(config.minio.endpoint, "http://localhost:9000");
        assert_eq!(config.crawl4ai.url, "http://localhost:11235");
        assert_eq!(config.gc_interval_hours, 24);
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
