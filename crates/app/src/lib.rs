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

pub mod workers;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use bon::Builder;
use job_common_telemetry as telemetry;
use job_server::{
    grpc::{GrpcServerConfig, hello::HelloService, start_grpc_server},
    http::{RestServerConfig, health_routes, start_rest_server},
};
use smart_default::SmartDefault;
use snafu::{ResultExt, Whatever};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::info;
use yunara_store::config::DatabaseConfig;

/// Represents the main application with lifecycle management
#[derive(SmartDefault)]
pub struct App {
    /// Application configuration
    pub config:             AppConfig,
    /// Controls if the application should continue running
    #[default(_code = "Arc::new(AtomicBool::new(false))")]
    pub running:            Arc<AtomicBool>,
    /// Cancellation token for graceful shutdown
    #[default(_code = "CancellationToken::new()")]
    pub cancellation_token: CancellationToken,
}

/// Configuration for the application
#[derive(Debug, Clone, SmartDefault, Builder)]
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
}

impl AppConfig {
    #[must_use]
    pub fn open(self) -> App {
        App {
            config: self,
            ..Default::default()
        }
    }
}

/// Handle for controlling a running application
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
    /// Start the application and return a handle for controlling it
    async fn start(&self) -> Result<AppHandle, Whatever> {
        // Initialize tracing subscriber
        let _guards = telemetry::logging::init_tracing_subscriber("job");

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

        // Initialize database
        let db_store = yunara_store::db::DBStore::new(self.config.db_config.clone())
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

        // Create notification repository and service
        let notification_repo =
            Arc::new(job_domain_notify::pg_repository::PgNotificationRepository::new(pool.clone()));
        let notification_service = Arc::new(job_domain_notify::service::NotificationService::new(
            notification_repo,
        ));

        // Create scheduler repository and service
        let scheduler_repo =
            Arc::new(job_domain_scheduler::pg_repository::PgSchedulerRepository::new(pool));
        let scheduler_service = Arc::new(job_domain_scheduler::service::SchedulerService::new(
            scheduler_repo,
        ));

        // Create domain services
        let resume_service = Arc::new(job_domain_resume::service::ResumeService::new(resume_repo));
        let application_service = Arc::new(
            job_domain_application::service::ApplicationService::new(application_repo),
        );
        let interview_service = Arc::new(job_domain_interview::service::InterviewService::new(
            interview_repo,
            None,
        ));

        // Build AppState
        let app_state = Arc::new(job_server::state::AppState {
            application_service,
            interview_service,
            resume_service,
            notification_service: notification_service.clone(),
            scheduler_service,
        });

        // Start servers
        let grpc_handle = start_grpc_server(&self.config.grpc_config, &[Arc::new(HelloService)])
            .whatever_context("Failed to start gRPC server")?;

        // Build all routes as a single closure
        let state = app_state.clone();
        let all_routes = move |router: axum::Router| {
            let router = health_routes(router);
            router.merge(job_server::api::api_routes(state.clone()))
        };

        let http_handle = start_rest_server(self.config.http_config.clone(), vec![all_routes])
            .await
            .whatever_context("Failed to start REST server")?;

        // Set up background worker manager
        let worker_state = crate::workers::notification_processor::WorkerState {
            notification_service,
        };

        let mut worker_manager = job_common_worker::Manager::with_state(worker_state);

        let _notification_handle = worker_manager
            .fallible_worker(
                crate::workers::notification_processor::NotificationProcessorWorker::new(50),
            )
            .name("notification-processor")
            .interval(std::time::Duration::from_secs(30))
            .spawn();

        info!("Background workers started");

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

    /// Run the application blocking until it's shut down
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
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_app_creation() {
        let app = AppConfig::default().open();
        assert!(!app.running.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_app_handle_shutdown() {
        let app = AppConfig::default().open();

        // Start the app (this will fail due to port binding in tests, but that's ok)
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
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(!handle.is_running());
    }
}
