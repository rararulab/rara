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

pub mod hello;

use std::sync::Arc;

use async_trait::async_trait;
use rsketch_base::readable_size::ReadableSize;
use rsketch_error::{ParseAddressSnafu, Result};
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use snafu::ResultExt;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tonic::{service::RoutesBuilder, transport::Server};
use tonic_health::server::HealthReporter;
use tonic_reflection::server::v1::{ServerReflection, ServerReflectionServer};
use tonic_tracing_opentelemetry::middleware::server::OtelGrpcLayer;
use tracing::info;

use crate::ServiceHandler;

/// Default maximum gRPC receiving message size (512 MB)
pub const DEFAULT_MAX_GRPC_RECV_MESSAGE_SIZE: ReadableSize = ReadableSize::mb(512);
/// Default maximum gRPC sending message size (512 MB)
pub const DEFAULT_MAX_GRPC_SEND_MESSAGE_SIZE: ReadableSize = ReadableSize::mb(512);

/// Configuration options for a gRPC server
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, SmartDefault, bon::Builder)]
pub struct GrpcServerConfig {
    /// The address to bind the gRPC server
    #[default = "127.0.0.1:50051"]
    pub bind_address:          String,
    /// The address to advertise to clients
    #[default = "127.0.0.1:50051"]
    pub server_address:        String,
    /// Maximum gRPC receiving (decoding) message size
    #[default(DEFAULT_MAX_GRPC_RECV_MESSAGE_SIZE)]
    pub max_recv_message_size: ReadableSize,
    /// Maximum gRPC sending (encoding) message size
    #[default(DEFAULT_MAX_GRPC_SEND_MESSAGE_SIZE)]
    pub max_send_message_size: ReadableSize,
}

/// Trait for gRPC service implementations that provides a standardized way to
/// register services with the gRPC server.
///
/// This trait abstracts the common patterns needed for gRPC services:
/// - Service registration with the tonic routes builder
/// - Reflection support through file descriptor sets
/// - Service identification for logging and monitoring
/// - Health status management
///
/// By implementing this trait, services can be easily integrated into the
/// `GrpcServer` framework, which handles server lifecycle, reflection setup,
/// health checking, and graceful shutdown automatically.
///
/// # Example
///
/// See `crates/server/src/grpc/hello.rs` for a complete implementation example.
#[async_trait]
pub trait GrpcServiceHandler: Send + Sync + 'static {
    /// The name of the service for logging and identification purposes
    fn service_name(&self) -> &'static str;
    /// The compiled protobuf file descriptor set used for gRPC reflection
    /// This should be generated using prost-build and included at compile time
    fn file_descriptor_set(&self) -> &'static [u8];
    /// Register the service implementation with the tonic routes builder
    /// This method should wrap the service in the appropriate tonic-generated
    /// server and add it to the builder
    fn register_service(self: &Arc<Self>, builder: &mut RoutesBuilder);
    /// `readiness_reporting` is called after the service is registered and
    /// allows the service to set its initial health status
    async fn readiness_reporting(
        self: &Arc<Self>,
        _cancellation_token: CancellationToken,
        health_reporter: HealthReporter,
    ) {
        // Default implementation does nothing - services can override this
        // to set their specific health status
        health_reporter
            .set_service_status("", tonic_health::ServingStatus::Serving)
            .await;
    }
}

/// Starts the gRPC server and returns a handle for managing its lifecycle.
///
/// This method:
/// 1. Sets up the gRPC reflection service using the service's file descriptor
///    set
/// 2. Sets up the health checking service if enabled
/// 3. Parses and binds to the configured address
/// 4. Spawns the server in a background task
/// 5. Returns a handle for lifecycle management
///
/// The server will automatically register the reflection service, health
/// service (if enabled), and the provided service implementation. It
/// supports graceful shutdown through the returned handle.
///
/// # Errors
/// Returns an error if the bind address cannot be parsed.
pub fn start_grpc_server(
    config: &GrpcServerConfig,
    services: &[Arc<impl GrpcServiceHandler>],
) -> Result<ServiceHandler> {
    // Parse bind address
    let bind_addr = config
        .bind_address
        .parse::<std::net::SocketAddr>()
        .context(ParseAddressSnafu {
            addr: config.bind_address.clone(),
        })?;

    let reflection_service = {
        let mut file_descriptor_sets = Vec::new();
        for service in services {
            file_descriptor_sets.push(service.file_descriptor_set());
        }
        file_descriptor_sets.push(tonic_reflection::pb::v1::FILE_DESCRIPTOR_SET);
        build_reflection_service(&file_descriptor_sets)
    };

    let (reporter, health_service) = tonic_health::server::health_reporter();
    let mut routes_builder = RoutesBuilder::default();
    routes_builder
        .add_service(health_service)
        .add_service(reflection_service);

    // register services
    for service in services {
        let service = service.clone();
        service.register_service(&mut routes_builder);
    }

    // Spawn the server task
    let cancellation_token = CancellationToken::new();
    let (join_handle, started_rx) = {
        let (started_tx, started_rx) = oneshot::channel::<()>();
        let cancellation_token_clone = cancellation_token.clone();
        let join_handle = tokio::spawn(async move {
            let result = Server::builder()
                .layer(OtelGrpcLayer::default())
                .accept_http1(true)
                .add_routes(routes_builder.routes())
                .serve_with_shutdown(bind_addr, async move {
                    info!("gRPC server (on {}) starting", bind_addr);
                    let _ = started_tx.send(());
                    info!("gRPC server (on {}) started", bind_addr);
                    cancellation_token_clone.cancelled().await;
                    info!("gRPC server (on {}) received shutdown signal", bind_addr);
                })
                .await;

            info!(
                "gRPC server (on {}) task completed: {:?}",
                bind_addr, result
            );
        });
        (join_handle, started_rx)
    };

    let reporter_handlers = {
        let mut handlers = Vec::new();
        for service in services {
            info!(
                "spawning readiness reporting task for {}",
                service.service_name()
            );
            let service = service.clone();
            let reporter = reporter.clone();
            let cancellation_token_clone = cancellation_token.clone();
            let handle = tokio::spawn(async move {
                service
                    .readiness_reporting(cancellation_token_clone, reporter)
                    .await;
                info!(
                    "readiness reporting task for {} completed",
                    service.service_name()
                );
            });
            handlers.push(handle);
        }
        handlers
    };

    let handle = ServiceHandler {
        join_handle,
        cancellation_token,
        started_rx: Some(started_rx),
        reporter_handles: reporter_handlers,
    };
    Ok(handle)
}

fn build_reflection_service(
    file_descriptor_sets: &[&[u8]],
) -> ServerReflectionServer<impl ServerReflection> {
    let mut builder = tonic_reflection::server::Builder::configure();

    for file_descriptor_set in file_descriptor_sets {
        builder = builder.register_encoded_file_descriptor_set(file_descriptor_set);
    }
    builder
        .build_v1()
        .expect("failed to build reflection service")
}
