use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
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
