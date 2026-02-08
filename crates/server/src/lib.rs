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

pub mod error;
pub mod grpc;
pub mod http;

use futures::future::join_all;
use rsketch_error::Result;
use tokio::{sync::oneshot::Receiver, task::JoinHandle};
use tokio_util::sync::CancellationToken;

/// Handle for managing a running service: grpc or http.
///
/// This handle provides control over a running service, allowing you to:
/// - Wait for the service to start accepting connections
/// - Signal graceful shutdown
/// - Wait for the service to fully stop
/// - Check if the service task has completed
///
/// The handle uses a cancellation token for graceful shutdown and provides
/// async methods for coordinating server lifecycle events.
pub struct ServiceHandler {
    /// Join handle for the server task
    join_handle:        JoinHandle<()>,
    /// Token for signalling shutdown
    cancellation_token: CancellationToken,
    /// Receiver for server start notification
    started_rx:         Option<Receiver<()>>,
    /// Join handles for readiness reporting tasks
    reporter_handles:   Vec<JoinHandle<()>>,
}

impl ServiceHandler {
    /// Waits for the server to start accepting connections.
    ///
    /// This method blocks until the server has successfully bound to its
    /// configured address and is ready to accept gRPC requests.
    ///
    /// # Panics
    /// Panics if called more than once, as the start signal is consumed.
    pub async fn wait_for_start(&mut self) -> Result<()> {
        self.started_rx
            .take()
            .expect("Server start signal already consumed")
            .await
            .expect("Failed to receive server start signal");
        Ok(())
    }

    /// Waits for the server to completely stop.
    ///
    /// This method consumes the handle and blocks until the server task
    /// has finished executing. Use this after calling `shutdown()` to
    /// ensure clean termination.
    ///
    /// # Panics
    /// Panics if the server task panicked during execution.
    pub async fn wait_for_stop(self) -> Result<()> {
        let handles = self
            .reporter_handles
            .into_iter()
            .chain(std::iter::once(self.join_handle));
        join_all(handles).await;
        Ok(())
    }

    /// Signals the server to begin graceful shutdown.
    ///
    /// This method triggers the shutdown process but does not wait for
    /// completion. Use `wait_for_stop()` to wait for the server to fully stop.
    pub fn shutdown(&self) { self.cancellation_token.cancel(); }

    /// Checks if the server task has completed.
    ///
    /// Returns `true` if the server has finished running, either due to
    /// shutdown or an error condition.
    #[must_use]
    pub fn is_finished(&self) -> bool { self.join_handle.is_finished() }
}
