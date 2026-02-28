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

//! Lazy-initializing mem0 client backed by an on-demand K8s pod.
//!
//! [`LazyMem0Client`] creates an ephemeral mem0 pod the first time a mem0
//! operation is requested (and an OpenAI API key has been configured). The
//! pod is kept alive for the application's lifetime and cleaned up on
//! shutdown.

use snafu::ResultExt;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::{
    error::{K8sSnafu, MemoryResult, NotConfiguredSnafu},
    mem0_client::Mem0Client,
    pod_manager::Mem0PodManager,
};

/// Internal state machine for the lazy client.
enum LazyState {
    /// No pod created yet (waiting for API key or first use).
    NotStarted,
    /// Pod is running and client is connected.
    Running {
        pod_name:  String,
        namespace: String,
        client:    Mem0Client,
    },
}

/// Lazy-initializing mem0 client that creates a K8s pod on first use.
///
/// # Lifecycle
///
/// 1. Created with [`new`](Self::new) — no pod exists yet.
/// 2. [`set_api_key`](Self::set_api_key) configures the OpenAI key.
/// 3. [`ensure_ready`](Self::ensure_ready) creates the pod on first call and
///    returns a cloned [`Mem0Client`].
/// 4. [`shutdown`](Self::shutdown) deletes the pod on application exit.
pub struct LazyMem0Client {
    pod_manager:    Mem0PodManager,
    state:          RwLock<LazyState>,
    openai_api_key: RwLock<Option<String>>,
    chroma_host:    String,
    chroma_port:    u16,
    image:          String,
    namespace:      String,
}

impl LazyMem0Client {
    /// Create a new lazy client. No pod is created until [`Self::ensure_ready`]
    /// is called.
    pub fn new(
        pod_manager: Mem0PodManager,
        chroma_host: String,
        chroma_port: u16,
        image: String,
        namespace: String,
    ) -> Self {
        Self {
            pod_manager,
            state: RwLock::new(LazyState::NotStarted),
            openai_api_key: RwLock::new(None),
            chroma_host,
            chroma_port,
            image,
            namespace,
        }
    }

    /// Set the OpenAI API key at runtime.
    ///
    /// Does **not** create the pod yet -- the pod is created lazily on the
    /// first mem0 operation via [`Self::ensure_ready`].
    pub async fn set_api_key(&self, key: String) {
        let mut guard = self.openai_api_key.write().await;
        *guard = Some(key);
        info!("mem0 API key configured; pod will be created on first use");
    }

    /// Returns `true` if an API key has been configured.
    pub async fn is_configured(&self) -> bool { self.openai_api_key.read().await.is_some() }

    /// Ensure the mem0 pod is running and return a cloned client.
    ///
    /// Creates the pod on first call (or after a previous pod was cleaned
    /// up). Subsequent calls return the cached client immediately.
    pub async fn ensure_ready(&self) -> MemoryResult<Mem0Client> {
        // Fast path: pod already running.
        {
            let state = self.state.read().await;
            if let LazyState::Running { client, .. } = &*state {
                return Ok(client.clone());
            }
        }

        // Slow path: need to create pod.
        let mut state = self.state.write().await;

        // Double-check after acquiring write lock.
        if let LazyState::Running { client, .. } = &*state {
            return Ok(client.clone());
        }

        let api_key = self.openai_api_key.read().await.clone().ok_or_else(|| {
            NotConfiguredSnafu {
                message: "mem0 API key not set",
            }
            .build()
        })?;

        info!("Creating mem0 pod on demand...");
        let (pod_name, pod_ip, port) = self
            .pod_manager
            .create_mem0_pod(
                &api_key,
                &self.chroma_host,
                self.chroma_port,
                &self.image,
                &self.namespace,
            )
            .await
            .context(K8sSnafu)?;

        let base_url = format!("http://{pod_ip}:{port}");
        info!(pod = %pod_name, url = %base_url, "mem0 pod is ready");

        let client = Mem0Client::new(base_url);
        *state = LazyState::Running {
            pod_name,
            namespace: self.namespace.clone(),
            client: client.clone(),
        };

        Ok(client)
    }

    /// Shut down the mem0 pod (graceful cleanup).
    pub async fn shutdown(&self) {
        let mut state = self.state.write().await;
        if let LazyState::Running {
            pod_name,
            namespace,
            ..
        } = &*state
        {
            info!(pod = %pod_name, "shutting down mem0 pod");
            if let Err(e) = self.pod_manager.delete_mem0_pod(pod_name, namespace).await {
                warn!(error = %e, "failed to delete mem0 pod during shutdown");
            }
        }
        *state = LazyState::NotStarted;
    }
}
