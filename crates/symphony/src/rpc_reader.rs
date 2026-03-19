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

//! Async reader that parses Ralph RPC JSON-lines from stdout.

use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    sync::mpsc,
    task::JoinHandle,
};

use crate::rpc::RpcEvent;

/// Spawn a task that reads JSON-lines from `reader` and sends parsed
/// [`RpcEvent`]s to `tx`. Unparseable lines are forwarded to `raw_tx` for
/// fallback logging. Returns the [`JoinHandle`] so callers can detect
/// panics or await completion.
pub fn spawn_rpc_reader<R: AsyncRead + Unpin + Send + 'static>(
    tx: mpsc::Sender<RpcEvent>,
    raw_tx: mpsc::Sender<String>,
    reader: R,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match serde_json::from_str::<RpcEvent>(&line) {
                Ok(event) => {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    tracing::debug!(line = %line, error = %err, "unparseable RPC line; forwarding to raw log");
                    let _ = raw_tx.send(line).await;
                }
            }
        }
        tracing::debug!("RPC reader task exiting");
    })
}
