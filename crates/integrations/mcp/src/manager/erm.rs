// Copyright 2025 Rararulab
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

use std::{collections::HashMap, sync::Arc};

use futures::future::BoxFuture;
use rmcp::model::{CreateElicitationResult, RequestId};
use tokio::sync::{Mutex, oneshot};

use crate::logging_client_handler::SendElicitation;

/// Manages pending MCP elicitation requests.
///
/// When an MCP server asks the user for input (e.g. OAuth consent or a form),
/// the request is parked here via a oneshot channel. The UI layer later
/// calls [`complete`](Self::complete) to deliver the user's response back
/// to the waiting server handler.
#[derive(Clone, Default)]
pub(crate) struct ElicitationRequestManager {
    inner: Arc<Mutex<ElicitationRequestManagerInner>>,
}

/// Interior state: maps `(server_name, request_id)` to the oneshot sender
/// that will unblock the waiting elicitation callback.
#[derive(Default)]
struct ElicitationRequestManagerInner {
    requests: HashMap<(String, RequestId), oneshot::Sender<CreateElicitationResult>>,
}

impl ElicitationRequestManager {
    /// Build a [`SendElicitation`] callback for a specific server.
    ///
    /// When the MCP server sends an elicitation request, the callback
    /// registers a oneshot channel and waits for a response (provided
    /// later via [`complete`](Self::complete)).
    pub(crate) fn make_sender(&self, server_name: String) -> SendElicitation {
        let inner = self.inner.clone();
        Box::new(move |request_id, _params| {
            let inner = inner.clone();
            let server_name = server_name.clone();
            Box::pin(async move {
                let (tx, rx) = oneshot::channel();
                {
                    let mut guard = inner.lock().await;
                    guard.requests.insert((server_name, request_id), tx);
                }
                rx.await
                    .map_err(|_| anyhow::anyhow!("elicitation response channel closed"))
            }) as BoxFuture<'static, anyhow::Result<CreateElicitationResult>>
        })
    }

    /// Deliver an elicitation response from the UI back to the waiting
    /// MCP server handler.
    pub(crate) async fn complete(
        &self,
        server_name: &str,
        request_id: RequestId,
        result: CreateElicitationResult,
    ) -> bool {
        let mut guard = self.inner.lock().await;
        if let Some(tx) = guard
            .requests
            .remove(&(server_name.to_string(), request_id))
        {
            tx.send(result).is_ok()
        } else {
            false
        }
    }
}
