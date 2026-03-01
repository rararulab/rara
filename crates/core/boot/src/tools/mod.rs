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

//! Primitive tool implementations and factory function.
//!
//! This module houses all **primitive tool** implementations and
//! provides [`default_primitives`] to obtain them in one call.

use std::sync::Arc;

use rara_domain_shared::settings::SettingsProvider;
use rara_kernel::tool::AgentToolRef;

mod bash;
mod composio;
mod edit_file;
mod find_files;
mod grep;
mod http_fetch;
mod list_directory;
#[cfg(feature = "k8s")]
pub mod pod;
mod read_file;
mod send_email;
mod storage_read;
mod write_file;

pub use bash::BashTool;
pub use composio::ComposioTool;
pub use edit_file::EditFileTool;
pub use find_files::FindFilesTool;
pub use grep::GrepTool;
pub use http_fetch::HttpFetchTool;
pub use list_directory::ListDirectoryTool;
#[cfg(feature = "k8s")]
pub use pod::PodTool;
pub use read_file::ReadFileTool;
pub use send_email::SendEmailTool;
pub use storage_read::StorageReadTool;
pub use write_file::WriteFileTool;

/// Dependencies required to construct primitive tools.
pub struct PrimitiveDeps {
    pub settings:               Arc<dyn SettingsProvider>,
    pub object_store:           opendal::Operator,
    pub composio_auth_provider: Arc<dyn rara_composio::ComposioAuthProvider>,
}

/// Returns all primitive tools, ready for registration.
pub fn default_primitives(deps: PrimitiveDeps) -> Vec<AgentToolRef> {
    let mut tools: Vec<AgentToolRef> = vec![
        // Core primitives
        Arc::new(BashTool::new()),
        Arc::new(ReadFileTool::new()),
        Arc::new(WriteFileTool::new()),
        Arc::new(EditFileTool::new()),
        Arc::new(FindFilesTool::new()),
        Arc::new(GrepTool::new()),
        Arc::new(ListDirectoryTool::new()),
        Arc::new(HttpFetchTool::new()),
        // Domain primitives
        Arc::new(SendEmailTool::new(deps.settings.clone())),
        Arc::new(StorageReadTool::new(deps.object_store)),
    ];
    tools.push(Arc::new(ComposioTool::from_auth_provider(
        deps.composio_auth_provider,
    )));
    tools
}
