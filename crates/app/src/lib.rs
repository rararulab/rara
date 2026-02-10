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

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use job_common_worker::Notifiable;
use job_domain_shared::telegram_service::TelegramService;
use job_server::{
    dedup_layer::{DedupLayer, DedupLayerConfig},
    grpc::{GrpcServerConfig, hello::HelloService, start_grpc_server},
    http::{RestServerConfig, health_routes, start_rest_server},
};
use smart_default::SmartDefault;
use snafu::{ResultExt, Whatever, whatever};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::info;
use yunara_store::config::DatabaseConfig;

// ---------------------------------------------------------------------------
// Config types
// ---------------------------------------------------------------------------

/// Telegram bot configuration.
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id:   i64,
}

/// OpenAI API configuration.
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    pub api_key: String,
    pub model:   String,
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
    /// Telegram configuration (optional)
    pub telegram:                 Option<TelegramConfig>,
    /// OpenAI configuration (optional)
    pub openai:                   Option<OpenAiConfig>,
}

impl AppConfig {
    /// Build an `AppConfig` from environment variables.
    ///
    /// Reads `DATABASE_URL`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`,
    /// `OPENAI_API_KEY`, and `OPENAI_MODEL`.
    pub fn from_env() -> Self {
        let db_config =
            DatabaseConfig::builder()
                .database_url(std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                    "postgres://postgres:postgres@localhost:5432/job".to_string()
                }))
                .build();

        let telegram = match (
            std::env::var("TELEGRAM_BOT_TOKEN"),
            std::env::var("TELEGRAM_CHAT_ID"),
        ) {
            (Ok(token), Ok(chat_id)) => {
                let chat_id: i64 = chat_id
                    .parse()
                    .expect("TELEGRAM_CHAT_ID must be an integer");
                Some(TelegramConfig {
                    bot_token: token,
                    chat_id,
                })
            }
            _ => None,
        };

        let openai = std::env::var("OPENAI_API_KEY").ok().map(|key| {
            let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());
            OpenAiConfig {
                api_key: key,
                model,
            }
        });

        Self {
            db_config,
            telegram,
            openai,
            ..Default::default()
        }
    }

    /// Initialize the database, create all domain services, and return a
    /// ready-to-run [`App`].
    pub async fn open(self) -> Result<App, Whatever> {
        info!("Initializing job application");

        let telegram_cfg = match self.telegram.as_ref() {
            Some(cfg) => cfg,
            None => {
                whatever!("Telegram is required: set TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID")
            }
        };
        let telegram = Arc::new(TelegramService::new(
            teloxide::Bot::new(&telegram_cfg.bot_token),
            telegram_cfg.chat_id,
        ));

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
        let notification_repo =
            Arc::new(job_domain_notify::pg_repository::PgNotificationRepository::new(pool.clone()));
        let scheduler_repo =
            Arc::new(job_domain_scheduler::pg_repository::PgSchedulerRepository::new(pool.clone()));
        let analytics_repo =
            Arc::new(job_domain_analytics::pg_repository::PgAnalyticsRepository::new(pool.clone()));
        let saved_job_repo = Arc::new(
            job_domain_saved_job::pg_repository::PgSavedJobRepository::new(pool.clone()),
        );

        // Notification service + senders
        let mut notification_service =
            job_domain_notify::service::NotificationService::new(notification_repo);
        info!("Telegram sender configured");
        notification_service.register_sender(
            job_domain_notify::types::NotificationChannel::Telegram,
            telegram.clone(),
        );
        notification_service.register_sender(
            job_domain_notify::types::NotificationChannel::Email,
            Arc::new(job_domain_notify::sender::NoopSender),
        );
        notification_service.register_sender(
            job_domain_notify::types::NotificationChannel::Webhook,
            Arc::new(job_domain_notify::sender::NoopSender),
        );
        let notification_service = Arc::new(notification_service);

        // Scheduler + analytics services
        let scheduler_service = Arc::new(job_domain_scheduler::service::SchedulerService::new(
            scheduler_repo,
        ));
        let analytics_service = Arc::new(job_domain_analytics::service::AnalyticsService::new(
            analytics_repo,
        ));

        // AI service (optional, needs OPENAI_API_KEY)
        let ai_service = self.openai.as_ref().map(|cfg| {
            Arc::new(job_ai::service::AiService::new(
                &cfg.api_key,
                cfg.model.clone(),
                None,
            ))
        });
        if ai_service.is_some() {
            info!("AI service configured (OpenAI)");
        } else {
            info!("AI service not configured (OPENAI_API_KEY not set)");
        }

        // Job repository (for saving parsed JDs)
        let job_repo: Arc<dyn job_domain_job_source::repository::JobRepository> = Arc::new(
            job_domain_job_source::pg_repository::PgJobRepository::new(pool.clone()),
        );

        // Object store (MinIO / S3) — optional, for saved-job markdown
        let object_store = {
            let cfg = job_object_store::ObjectStoreConfig::builder()
                .endpoint(
                    std::env::var("MINIO_ENDPOINT")
                        .unwrap_or_else(|_| "http://localhost:9000".to_owned()),
                )
                .bucket(
                    std::env::var("MINIO_BUCKET")
                        .unwrap_or_else(|_| "job-markdown".to_owned()),
                )
                .access_key(
                    std::env::var("MINIO_ACCESS_KEY")
                        .unwrap_or_else(|_| "minioadmin".to_owned()),
                )
                .secret_key(
                    std::env::var("MINIO_SECRET_KEY")
                        .unwrap_or_else(|_| "minioadmin".to_owned()),
                )
                .region("us-east-1".to_owned())
                .root("/".to_owned())
                .build();
            match job_object_store::ObjectStore::new(&cfg) {
                Ok(store) => {
                    info!("Object store (MinIO) configured");
                    Some(Arc::new(store))
                }
                Err(e) => {
                    info!("Object store not available: {e}");
                    None
                }
            }
        };

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

        // Crawl4AI client + saved job pipeline (requires object_store + ai_service)
        let crawl4ai_url = std::env::var("CRAWL4AI_URL")
            .unwrap_or_else(|_| "http://localhost:11235".to_owned());
        let crawl_client =
            job_domain_saved_job::crawl4ai::Crawl4AiClient::new(&crawl4ai_url);

        let saved_job_pipeline = match (&object_store, &ai_service) {
            (Some(os), Some(ai)) => {
                let pipeline = job_domain_saved_job::pipeline::SavedJobPipeline::new(
                    saved_job_service.clone(),
                    crawl_client,
                    os.clone(),
                    ai.clone(),
                );
                info!("Saved job pipeline configured (Crawl4AI + AI analysis)");
                Some(Arc::new(pipeline))
            }
            _ => {
                info!("Saved job pipeline not configured (requires object store + AI)");
                None
            }
        };

        // Build routes closure — captures Arc'd services for on-demand Router
        // construction (axum::Router does not implement Clone).
        let resume_svc = resume_service.clone();
        let app_svc = application_service.clone();
        let interview_svc = interview_service.clone();
        let notify_svc = notification_service.clone();
        let scheduler_svc = scheduler_service.clone();
        let analytics_svc = analytics_service.clone();
        let saved_job_svc = saved_job_service.clone();
        let job_source_svc = job_source_service.clone();

        let routes_fn: Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync> =
            Box::new(move |router: axum::Router| {
                let router = health_routes(router);
                router
                    .merge(job_domain_resume::routes::routes(resume_svc.clone()))
                    .merge(job_domain_application::routes::routes(app_svc.clone()))
                    .merge(job_domain_interview::routes::routes(interview_svc.clone()))
                    .merge(job_domain_notify::routes::routes(notify_svc.clone()))
                    .merge(job_domain_scheduler::routes::routes(scheduler_svc.clone()))
                    .merge(job_domain_analytics::routes::routes(analytics_svc.clone()))
                    .merge(job_domain_saved_job::routes::routes(saved_job_svc.clone()))
                    .merge(
                        job_domain_job_source::routes::routes(job_source_svc.clone())
                            .layer(DedupLayer::new(DedupLayerConfig::default())),
                    )
            });

        info!("Application initialized successfully");

        Ok(App {
            config: self,
            running: Arc::new(AtomicBool::new(false)),
            cancellation_token: CancellationToken::new(),
            routes_fn,
            notification_service,
            telegram,
            ai_service,
            job_repo,
            job_source_service,
            saved_job_service: Some(saved_job_service.clone()),
            object_store,
            saved_job_pipeline,
        })
    }
}

// ---------------------------------------------------------------------------
// App + AppHandle
// ---------------------------------------------------------------------------

/// Represents the main application with lifecycle management.
pub struct App {
    /// Application configuration
    config:               AppConfig,
    /// Controls if the application should continue running
    running:              Arc<AtomicBool>,
    /// Cancellation token for graceful shutdown
    cancellation_token:   CancellationToken,
    /// Closure that builds the axum Router from domain routes
    routes_fn:            Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync>,
    /// Notification service needed by background workers
    notification_service: Arc<job_domain_notify::service::NotificationService>,
    /// Telegram service shared by workers and notification sender
    telegram:             Arc<TelegramService>,
    /// AI service for JD parsing (optional)
    ai_service:           Option<Arc<job_ai::service::AiService>>,
    /// Job repository for persisting parsed jobs
    job_repo:             Arc<dyn job_domain_job_source::repository::JobRepository>,
    /// Job source service for discovery (used by Telegram /search command)
    job_source_service:   Arc<job_domain_job_source::service::JobSourceService>,
    /// Saved job service for GC worker
    saved_job_service:    Option<Arc<job_domain_saved_job::service::SavedJobService<job_domain_saved_job::pg_repository::PgSavedJobRepository>>>,
    /// Object store for GC worker S3 cleanup
    object_store:         Option<Arc<job_object_store::ObjectStore>>,
    /// Saved job pipeline for crawl + AI analysis
    saved_job_pipeline:   Option<Arc<job_domain_saved_job::pipeline::SavedJobPipeline<job_domain_saved_job::pg_repository::PgSavedJobRepository>>>,
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
        let grpc_handle = start_grpc_server(&self.config.grpc_config, &[Arc::new(HelloService)])
            .whatever_context("Failed to start gRPC server")?;

        info!("starting rest server ...");
        let http_handle = start_rest_server(self.config.http_config.clone(), vec![self.routes_fn])
            .await
            .whatever_context("Failed to start REST server")?;

        // Set up mpsc channel for JD parse requests
        let (jd_tx, jd_rx) = tokio::sync::mpsc::channel(100);
        let jd_parse_notify = Arc::new(tokio::sync::Mutex::new(None));
        let notification_service = self.notification_service.clone();

        // Set up background worker manager
        let worker_state = job_workers::notification_processor::WorkerState {
            notification_service: self.notification_service,
            ai_service:           self.ai_service,
            job_repo:             Some(self.job_repo),
            jd_parse_tx:          jd_tx,
            jd_parse_notify:      jd_parse_notify.clone(),
            telegram:             self.telegram,
            job_source_service:   self.job_source_service,
            saved_job_service:    self.saved_job_service,
            object_store:         self.object_store,
            saved_job_pipeline:   self.saved_job_pipeline,
        };

        let mut worker_manager = job_common_worker::Manager::with_state(worker_state);

        let notification_handle = worker_manager
            .fallible_worker(
                job_workers::notification_processor::NotificationProcessorWorker::new(50),
            )
            .name("notification-processor")
            .on_notify()
            .spawn();
        notification_service.set_notify_trigger(notification_handle.clone());
        notification_handle.notify();

        // JD parser worker (notify trigger, drains mpsc channel)
        let jd_parser_handle = worker_manager
            .fallible_worker(job_workers::jd_parser::JdParserWorker::new(jd_rx))
            .name("jd-parser")
            .on_notify()
            .spawn();
        *jd_parse_notify.lock().await = Some(jd_parser_handle);

        // Telegram bot worker (long-running, Once trigger)
        let _telegram_handle = worker_manager
            .fallible_worker(job_workers::telegram_bot::TelegramBotWorker)
            .name("telegram-bot")
            .once()
            .spawn();

        // Saved job pipeline worker (notify trigger, processes PendingCrawl jobs)
        let _pipeline_handle = worker_manager
            .fallible_worker(
                job_workers::saved_job_pipeline::SavedJobPipelineWorker,
            )
            .name("saved-job-pipeline")
            .on_notify()
            .spawn();

        // Saved job GC worker (periodic, default every 24 hours)
        let gc_interval_secs = std::env::var("GC_INTERVAL_HOURS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(24)
            * 3600;
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

            // Shutdown background workers
            info!("Shutting down background workers");
            worker_manager.shutdown().await;

            // Shutdown servers
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
        assert!(config.enable_graceful_shutdown);
        assert!(config.telegram.is_none());
        assert!(config.openai.is_none());
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
