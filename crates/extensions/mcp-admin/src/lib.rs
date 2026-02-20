mod error;
mod router;
mod types;

use rara_mcp::manager::mgr::McpManager;

pub fn router(manager: McpManager) -> axum::Router {
    router::mcp_router(manager)
}
