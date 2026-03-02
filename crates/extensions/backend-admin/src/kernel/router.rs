use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    routing::get,
};
use rara_kernel::Kernel;
use rara_kernel::audit::AuditFilter;
use serde::Deserialize;

use super::problem::ProblemDetails;

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    pub limit: usize,
}

fn default_audit_limit() -> usize {
    50
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn kernel_routes(kernel: Arc<Kernel>) -> Router {
    Router::new()
        .route("/api/v1/kernel/stats", get(get_stats))
        .route("/api/v1/kernel/processes", get(list_processes))
        .route(
            "/api/v1/kernel/processes/{agent_id}/turns",
            get(get_process_turns),
        )
        .route(
            "/api/v1/kernel/processes/{agent_id}/stream",
            get(stream_process),
        )
        .route("/api/v1/kernel/approvals", get(list_approvals))
        .route("/api/v1/kernel/audit", get(query_audit))
        .with_state(kernel)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn get_stats(
    State(kernel): State<Arc<Kernel>>,
) -> Result<Json<rara_kernel::process::SystemStats>, ProblemDetails> {
    Ok(Json(kernel.system_stats()))
}

async fn list_processes(
    State(kernel): State<Arc<Kernel>>,
) -> Result<Json<Vec<rara_kernel::process::ProcessStats>>, ProblemDetails> {
    Ok(Json(kernel.list_processes().await))
}

async fn get_process_turns(
    State(kernel): State<Arc<Kernel>>,
    Path(agent_id): Path<String>,
) -> Result<Json<Vec<rara_kernel::agent_turn::TurnTrace>>, ProblemDetails> {
    let aid = rara_kernel::process::AgentId(
        uuid::Uuid::parse_str(&agent_id)
            .map_err(|e| ProblemDetails::bad_request(format!("invalid agent_id: {e}")))?,
    );
    // Verify the process exists before returning traces.
    if kernel.process_table().get(aid).is_none() {
        return Err(ProblemDetails::not_found(
            "Process Not Found",
            format!("process not found: {agent_id}"),
        ));
    }
    Ok(Json(kernel.get_process_turns(aid)))
}

async fn list_approvals(
    State(kernel): State<Arc<Kernel>>,
) -> Result<Json<Vec<rara_kernel::approval::ApprovalRequest>>, ProblemDetails> {
    Ok(Json(kernel.approval().list_pending()))
}

async fn query_audit(
    State(kernel): State<Arc<Kernel>>,
    Query(params): Query<AuditQuery>,
) -> Result<Json<Vec<rara_kernel::audit::AuditEvent>>, ProblemDetails> {
    let filter = AuditFilter {
        limit: params.limit,
        ..Default::default()
    };
    Ok(Json(kernel.audit_query(filter).await))
}

async fn stream_process(
    State(kernel): State<Arc<Kernel>>,
    Path(agent_id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<impl axum::response::IntoResponse, ProblemDetails> {
    let aid = rara_kernel::process::AgentId(
        uuid::Uuid::parse_str(&agent_id)
            .map_err(|e| ProblemDetails::bad_request(format!("invalid agent_id: {e}")))?,
    );

    let process = kernel
        .process_table()
        .get(aid)
        .ok_or_else(|| {
            ProblemDetails::not_found(
                "Process Not Found",
                format!("process not found: {agent_id}"),
            )
        })?;
    let session_id = process.session_id.clone();
    let stream_hub = kernel.stream_hub().clone();

    Ok(ws.on_upgrade(move |socket| {
        handle_process_stream(socket, stream_hub, session_id)
    }))
}

async fn handle_process_stream(
    mut socket: WebSocket,
    stream_hub: std::sync::Arc<rara_kernel::io::stream::StreamHub>,
    session_id: rara_kernel::process::SessionId,
) {
    use tokio::time::Duration;

    let mut poll_interval = tokio::time::interval(Duration::from_millis(200));
    let mut receivers: Vec<
        tokio::sync::broadcast::Receiver<rara_kernel::io::stream::StreamEvent>,
    > = Vec::new();

    loop {
        // If no active receivers, try to subscribe
        if receivers.is_empty() {
            let subs = stream_hub.subscribe_session(&session_id);
            receivers = subs.into_iter().map(|(_, rx)| rx).collect();
            if receivers.is_empty() {
                // No active stream — wait and retry
                tokio::select! {
                    _ = poll_interval.tick() => continue,
                    msg = socket.recv() => {
                        match msg {
                            Some(Ok(Message::Close(_))) | None => return,
                            _ => continue,
                        }
                    }
                }
            }
        }

        // Drain from all receivers
        let mut got_event = false;
        for rx in &mut receivers {
            match rx.try_recv() {
                Ok(event) => {
                    got_event = true;
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    let _ = socket
                        .send(Message::Text(r#"{"type":"done"}"#.into()))
                        .await;
                    return;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "process stream subscriber lagged");
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
            }
        }

        if !got_event {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(50)) => {},
                msg = socket.recv() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => return,
                        _ => {}
                    }
                }
            }
        }
    }
}
