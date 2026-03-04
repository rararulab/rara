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

//! Kernel-side application state — holds infrastructure and tool registries
//! needed to boot a [`Kernel`](rara_kernel::Kernel).
//!
//! [`RaraState`] is the kernel-dependency half of the old `AppState` god
//! object.  It initializes LLM providers, tools, memory, skills, MCP, and
//! the session repository but does **not** create a `Kernel` itself — the
//! caller does that in the app crate using the fields exposed here.

use std::sync::Arc;

use opendal::Operator;
use snafu::{ResultExt, Whatever};
use tracing::info;

/// Kernel-side application state.
///
/// Owns everything the kernel needs to run: provider registry, tool registry,
/// memory, skills, MCP, coding tasks, user store, and session repository.
///
/// Does NOT hold a `Kernel` — the app crate builds one from these fields.
#[derive(Clone)]
pub struct RaraState {
    pub credential_store:    rara_keyring_store::KeyringStoreRef,
    pub driver_registry:     Arc<rara_kernel::llm::DriverRegistry>,
    pub tool_registry:       Arc<rara_kernel::tool::ToolRegistry>,
    pub user_store:          Arc<dyn rara_kernel::process::user::UserStore>,
    pub session_repo:        Arc<dyn rara_sessions::repository::SessionRepository>,
    pub memory_manager:      Arc<rara_memory::MemoryManager>,
    pub skill_registry:      rara_skills::registry::InMemoryRegistry,
    pub mcp_manager:         rara_mcp::manager::mgr::McpManager,
    pub coding_task_service: rara_coding_task::service::CodingTaskService,
    pub object_store:        Operator,
    pub settings_provider:   Arc<dyn rara_domain_shared::settings::SettingsProvider>,
}

impl RaraState {
    /// Initialize all kernel-side infrastructure.
    ///
    /// This mirrors the kernel-related initialization that was previously in
    /// `AppState::init()` in the workers crate.
    pub async fn init(
        pool: sqlx::PgPool,
        object_store: Operator,
        settings_provider: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
        mem0_base_url: String,
        memos_base_url: String,
        memos_token: String,
        hindsight_base_url: String,
        hindsight_bank_id: String,
    ) -> Result<Self, Whatever> {
        // -- credential store --------------------------------------------------

        let credential_store: rara_keyring_store::KeyringStoreRef =
            Arc::new(rara_pg_credential_store::PgKeyringStore::new(pool.clone()));

        // -- LLM driver registry ----------------------------------------------

        let driver_registry = crate::llm_registry::build_driver_registry(
            settings_provider.clone(),
            &*credential_store,
        )
        .await
        .whatever_context("Failed to initialize LLM driver registry")?;

        {
            let driver_registry_ref = driver_registry.clone();
            let settings_ref = settings_provider.clone();
            let credential_store_ref = credential_store.clone();
            tokio::spawn(async move {
                let mut rx = settings_ref.subscribe();
                while rx.changed().await.is_ok() {
                    match crate::llm_registry::build_driver_registry(
                        settings_ref.clone(),
                        &*credential_store_ref,
                    )
                    .await
                    {
                        Ok(updated) => {
                            driver_registry_ref.update(updated.as_ref());
                            info!("LLM driver registry reloaded from settings");
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "Failed to reload LLM driver registry from settings");
                        }
                    }
                }
            });
        }

        // -- session repository -----------------------------------------------

        let session_repo: Arc<dyn rara_sessions::repository::SessionRepository> = Arc::new(
            rara_sessions::pg_repository::PgSessionRepository::new(
                pool.clone(),
                rara_paths::sessions_dir(),
            )
            .await
            .whatever_context("Failed to initialize session repository")?,
        );

        // -- Composio auth provider -------------------------------------------

        let composio_auth_provider: Arc<dyn rara_composio::ComposioAuthProvider> = Arc::new(
            crate::composio::SettingsComposioAuthProvider::new(settings_provider.clone()),
        );

        // -- primitive tools (Layer 1) ----------------------------------------

        let mut tool_registry = rara_kernel::tool::ToolRegistry::new();
        for tool in crate::tools::default_primitives(crate::tools::PrimitiveDeps {
            settings: settings_provider.clone(),
            object_store: object_store.clone(),
            composio_auth_provider,
        }) {
            tool_registry.register_primitive(tool);
        }

        // -- memory -----------------------------------------------------------

        let memory_manager = crate::memory::init_memory_manager(
            mem0_base_url,
            memos_base_url,
            memos_token,
            hindsight_base_url,
            hindsight_bank_id,
        );
        let recall_engine = crate::memory::init_recall_engine();

        // -- coding task service ----------------------------------------------

        let default_repo_url = std::env::var("RARA_DEFAULT_REPO_URL")
            .unwrap_or_else(|_| "https://github.com/rararulab/rara".to_owned());
        let coding_task_service = crate::coding_task::init_coding_task_service(
            pool.clone(),
            settings_provider.clone(),
            default_repo_url,
        );

        // -- skills registry --------------------------------------------------

        let skill_registry = crate::skills::init_skill_registry(pool.clone());

        // -- MCP manager ------------------------------------------------------

        let mcp_manager = crate::mcp::init_mcp_manager(credential_store.clone())
            .await
            .whatever_context("Failed to initialize MCP manager")?;

        // -- service tools (Layer 2) ------------------------------------------

        crate::tools::register_service_tools(
            &mut tool_registry,
            crate::tools::ServiceToolDeps {
                memory_manager: memory_manager.clone(),
                recall_engine,
                coding_task_service: coding_task_service.clone(),
                skill_registry: skill_registry.clone(),
                mcp_manager: mcp_manager.clone(),
            },
        );

        let tools = Arc::new(tool_registry);

        // -- user store -------------------------------------------------------

        let user_store: Arc<dyn rara_kernel::process::user::UserStore> =
            Arc::new(crate::user_store::PgUserStore::new(pool.clone()));
        crate::user_store::ensure_default_users(&pool)
            .await
            .whatever_context("Failed to ensure default users")?;

        info!("RaraState initialized");

        Ok(Self {
            credential_store,
            driver_registry,
            tool_registry: tools,
            user_store,
            session_repo,
            memory_manager,
            skill_registry,
            mcp_manager,
            coding_task_service,
            object_store,
            settings_provider,
        })
    }
}
