//! Composio tool suite.
//!
//! Four focused tools replacing the former monolithic `composio` meta-tool:
//! - `composio_list`     — discover available actions
//! - `composio_execute`  — run an action
//! - `composio_connect`  — get OAuth connection URL
//! - `composio_accounts` — list connected accounts

mod accounts;
mod connect;
mod execute;
mod list;
mod shared;

use std::sync::Arc;

use rara_composio::ComposioAuthProvider;
use rara_kernel::tool::AgentToolRef;

use self::{
    accounts::ComposioAccountsTool, connect::ComposioConnectTool, execute::ComposioExecuteTool,
    list::ComposioListTool,
};

/// Build all four Composio tools from a shared auth provider.
pub fn build_tools(auth_provider: Arc<dyn ComposioAuthProvider>) -> [AgentToolRef; 4] {
    let shared = shared::ComposioShared::from_auth_provider(auth_provider);
    [
        Arc::new(ComposioListTool::new(shared.clone())),
        Arc::new(ComposioExecuteTool::new(shared.clone())),
        Arc::new(ComposioConnectTool::new(shared.clone())),
        Arc::new(ComposioAccountsTool::new(shared)),
    ]
}
