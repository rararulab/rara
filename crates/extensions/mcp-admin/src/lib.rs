mod error;
mod router;
mod types;

use std::sync::Arc;

use rara_mcp::manager::mgr::McpManager;

pub fn router(manager: Arc<McpManager>) -> axum::Router {
    router::mcp_router(manager)
}
