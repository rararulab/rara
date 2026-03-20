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

//! Kernel bootstrap — assembles all kernel dependencies in one shot.
//!
//! Consolidates the old `rara-boot` crate (state, llm_registry, user_store,
//! resolvers, manifests, mcp, composio, skills) into a single module with
//! private helpers and a public `boot()` entry point.

use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Whatever};
use tracing::info;

// =========================================================================
// Public types
// =========================================================================

/// Result of the boot sequence — everything the caller needs to build a
/// [`Kernel`](rara_kernel::kernel::Kernel).
#[derive(Clone)]
pub(crate) struct BootResult {
    pub credential_store:     rara_keyring_store::KeyringStoreRef,
    pub driver_registry:      Arc<rara_kernel::llm::DriverRegistry>,
    pub tool_registry:        Arc<rara_kernel::tool::ToolRegistry>,
    pub user_store:           Arc<dyn rara_kernel::identity::UserStore>,
    pub session_index:        Arc<dyn rara_kernel::session::SessionIndex>,
    pub tape_service:         rara_kernel::memory::TapeService,
    pub skill_registry:       rara_skills::registry::InMemoryRegistry,
    pub mcp_manager:          rara_mcp::manager::mgr::McpManager,
    pub output_interceptor:   rara_kernel::tool::DynamicOutputInterceptor,
    pub settings_provider:    Arc<dyn rara_domain_shared::settings::SettingsProvider>,
    pub identity_resolver:    Arc<dyn rara_kernel::io::IdentityResolver>,
    pub agent_registry:       Arc<rara_kernel::agent::AgentRegistry>,
    /// Handle reference for `DispatchRaraTool` — must be wired with a
    /// `KernelHandle` after kernel startup.
    pub dispatch_rara_handle:
        std::sync::Arc<tokio::sync::RwLock<Option<rara_kernel::handle::KernelHandle>>>,
    /// Handle reference for `ListSessionsTool` — must be wired with a
    /// `KernelHandle` after kernel startup.
    pub list_sessions_handle:
        std::sync::Arc<tokio::sync::RwLock<Option<rara_kernel::handle::KernelHandle>>>,
    /// Knowledge layer service for long-term memory.
    pub knowledge_service:    rara_kernel::memory::knowledge::KnowledgeServiceRef,
    /// Default provider's model lister for `/v1/models` queries.
    pub model_lister:         rara_kernel::llm::LlmModelListerRef,
    /// Shared mutation sink for dock tools — passed to both tools and routes.
    pub dock_mutation_sink:   rara_dock::DockMutationSink,
}

/// A user entry in the YAML configuration file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub name:      String,
    /// `"root"` | `"admin"` | `"user"`
    pub role:      String,
    #[serde(default)]
    pub platforms: Vec<PlatformBindingConfig>,
}

/// A platform identity binding for a configured user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformBindingConfig {
    /// Channel type: `"telegram"`, `"web"`, `"cli"`, etc.
    #[serde(rename = "type")]
    pub channel_type: String,
    /// Platform-side user identifier (e.g. Telegram user ID).
    pub user_id:      String,
}

// =========================================================================
// boot() — main entry point
// =========================================================================

/// Initialize all kernel-side infrastructure and return a [`BootResult`].
pub(crate) async fn boot(
    pool: sqlx::SqlitePool,
    settings_provider: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
    users: &[UserConfig],
) -> Result<BootResult, Whatever> {
    // -- credential store --------------------------------------------------

    let credential_store: rara_keyring_store::KeyringStoreRef =
        Arc::new(rara_pg_credential_store::PgKeyringStore::new(pool.clone()));

    // -- LLM driver registry -----------------------------------------------

    let driver_registry = build_driver_registry(settings_provider.clone(), &*credential_store)
        .await
        .whatever_context("Failed to initialize LLM driver registry")?;

    {
        let driver_registry_ref = driver_registry.clone();
        let settings_ref = settings_provider.clone();
        let credential_store_ref = credential_store.clone();
        tokio::spawn(async move {
            let mut rx = settings_ref.subscribe();
            while rx.changed().await.is_ok() {
                match build_driver_registry(settings_ref.clone(), &*credential_store_ref).await {
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

    // -- session index (tape-centric) --------------------------------------

    let session_index: Arc<dyn rara_kernel::session::SessionIndex> = Arc::new(
        rara_sessions::file_index::FileSessionIndex::new(rara_paths::sessions_dir().join("index"))
            .await
            .whatever_context("Failed to initialize file session index")?,
    );
    info!("FileSessionIndex initialized");

    // -- tape store --------------------------------------------------------

    let workspace_path = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let tape_service = rara_kernel::memory::TapeService::new(
        rara_kernel::memory::FileTapeStore::new(rara_paths::memory_dir(), &workspace_path)
            .await
            .whatever_context("Failed to initialize FileTapeStore")?,
    );
    info!("TapeService initialized");

    // -- Composio auth provider --------------------------------------------

    let composio_auth_provider: Arc<dyn rara_composio::ComposioAuthProvider> =
        Arc::new(SettingsComposioAuthProvider::new(settings_provider.clone()));

    // -- skills registry ---------------------------------------------------

    let skill_registry = rara_skills::registry::InMemoryRegistry::new();
    rara_skills::cache::spawn_background_sync(pool.clone(), skill_registry.clone());
    info!("skill registry initialized with background sync");

    // -- marketplace service -----------------------------------------------

    let marketplace_service = Arc::new(
        rara_skills::marketplace::MarketplaceService::new().with_registry(skill_registry.clone()),
    );
    info!("marketplace service initialized");
    let clawhub_client = Arc::new(rara_skills::clawhub::ClawhubClient::new());
    info!("clawhub client initialized");

    // -- MCP manager -------------------------------------------------------

    let mcp_manager = init_mcp_manager(credential_store.clone())
        .await
        .whatever_context("Failed to initialize MCP manager")?;

    // -- ACP agent registry ------------------------------------------------

    let acp_registry = init_acp_registry().await?;

    // -- tools -------------------------------------------------------------

    let dock_mutation_sink = rara_dock::DockMutationSink::new();

    let mut tool_registry = rara_kernel::tool::ToolRegistry::new();
    let tool_result = crate::tools::register_all(
        &mut tool_registry,
        crate::tools::ToolDeps {
            settings: settings_provider.clone(),
            composio_auth_provider,
            skill_registry: skill_registry.clone(),
            mcp_manager: mcp_manager.clone(),
            tape_service: tape_service.clone(),
            session_index: session_index.clone(),
            marketplace_service: marketplace_service.clone(),
            clawhub_client: clawhub_client.clone(),
            dock_mutation_sink: dock_mutation_sink.clone(),
            acp_registry,
        },
    );

    // -- browser subsystem -------------------------------------------------

    // match rara_kernel::browser::BrowserManager::start(rara_kernel::browser::BrowserConfig::default())
    //     .await
    // {
    //     Ok(manager) => {
    //         let manager_ref: rara_kernel::browser::BrowserManagerRef =
    //             std::sync::Arc::new(manager);
    //         for tool in rara_kernel::tool::browser::browser_tools(manager_ref) {
    //             tool_registry.register(tool);
    //         }
    //         info!("Browser subsystem initialized with Lightpanda");
    //     }
    //     Err(e) => {
    //         warn!("Browser subsystem disabled: {e}");
    //     }
    // }

    // -- output interceptor (context-mode) --------------------------------
    // Built after tools so we can derive the bypass set from the registry.

    let bypass_set: std::collections::HashSet<String> = tool_registry
        .iter()
        .filter(|(_, tool)| tool.bypass_output_interceptor())
        .map(|(name, _)| name.to_owned())
        .collect();

    let output_interceptor: rara_kernel::tool::DynamicOutputInterceptor = {
        let status = mcp_manager.server_connection_status("context-mode").await;
        if status == rara_mcp::manager::mgr::ConnectionStatus::Connected {
            info!("context-mode connected, enabling output interceptor");
            Arc::new(tokio::sync::RwLock::new(Some(Arc::new(
                crate::context_mode::ContextModeInterceptor::new(mcp_manager.clone())
                    .with_bypass_set(bypass_set),
            ))))
        } else {
            info!("context-mode not available, output interceptor disabled");
            Arc::new(tokio::sync::RwLock::new(None))
        }
    };

    let tools = Arc::new(tool_registry);

    // -- user store --------------------------------------------------------

    let user_store: Arc<dyn rara_kernel::identity::UserStore> =
        Arc::new(InMemoryUserStore::from_config(users));

    // -- identity resolver -------------------------------------------------

    let identity_resolver: Arc<dyn rara_kernel::io::IdentityResolver> =
        Arc::new(PlatformIdentityResolver::new(users));

    // -- agent registry ----------------------------------------------------

    let agent_registry = Arc::new(load_default_registry());

    // -- default provider model lister / embedder ----------------------------

    let default_provider = {
        use rara_domain_shared::settings::keys;
        settings_provider
            .get_first(&[keys::LLM_DEFAULT_PROVIDER, keys::LLM_PROVIDER])
            .await
            .unwrap_or_else(|| "openrouter".to_owned())
    };
    let default_driver: Arc<rara_kernel::llm::OpenAiDriver> = Arc::new(
        rara_kernel::llm::OpenAiDriver::from_settings(settings_provider.clone(), &default_provider),
    );
    let model_lister: rara_kernel::llm::LlmModelListerRef = default_driver.clone();
    let embedder: rara_kernel::llm::LlmEmbedderRef = default_driver;

    // -- knowledge layer ------------------------------------------------------

    let knowledge_service = init_knowledge_service(pool, settings_provider.as_ref(), embedder)
        .await
        .whatever_context("Failed to initialize knowledge layer")?;

    info!("Boot completed");

    Ok(BootResult {
        credential_store,
        driver_registry,
        tool_registry: tools,
        user_store,
        session_index,
        tape_service,
        skill_registry,
        mcp_manager,
        output_interceptor,
        settings_provider,
        identity_resolver,
        agent_registry,
        dispatch_rara_handle: tool_result.dispatch_rara_handle,
        list_sessions_handle: tool_result.list_sessions_handle,
        knowledge_service,
        dock_mutation_sink,
        model_lister,
    })
}

// =========================================================================
// Private: LLM driver registry
// =========================================================================

/// Build a [`DriverRegistry`](rara_kernel::llm::DriverRegistry) from
/// runtime settings.
async fn build_driver_registry(
    settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
    credential_store: &dyn rara_keyring_store::KeyringStore,
) -> anyhow::Result<Arc<rara_kernel::llm::DriverRegistry>> {
    use rara_domain_shared::settings::keys;
    use rara_kernel::llm::{DriverRegistry, OpenAiDriver};

    let default_provider = settings
        .as_ref()
        .get_first(&[keys::LLM_DEFAULT_PROVIDER, keys::LLM_PROVIDER])
        .await
        .ok_or_else(|| {
            anyhow::anyhow!(
                "LLM default provider is not configured (checked: {}, {})",
                keys::LLM_DEFAULT_PROVIDER,
                keys::LLM_PROVIDER
            )
        })?;

    let registry = Arc::new(DriverRegistry::new(&default_provider));

    // -- auto-discover providers from settings --------------------------------

    let all_settings = settings.list().await;
    let provider_names: BTreeSet<&str> = all_settings
        .keys()
        .filter_map(|k| k.strip_prefix("llm.providers."))
        .filter_map(|k| k.split('.').next())
        .collect();

    for &name in &provider_names {
        registry.register_driver(
            name,
            Arc::new(OpenAiDriver::from_settings(settings.clone(), name)),
        );

        // Read per-provider default_model and fallback_models
        let model_key = format!("llm.providers.{name}.default_model");
        if let Some(default_model) = all_settings
            .get(&model_key)
            .filter(|v| !v.trim().is_empty())
        {
            let fallback_key = format!("llm.providers.{name}.fallback_models");
            let fallback_models: Vec<String> = all_settings
                .get(&fallback_key)
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            registry.set_provider_model(name, default_model, fallback_models);
        }
    }

    // Validate that default_provider has a default_model configured
    let default_model_key = format!("llm.providers.{default_provider}.default_model");
    let default_model = all_settings
        .get(&default_model_key)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "LLM default model is not configured for provider '{default_provider}' (checked: \
                 {default_model_key})"
            )
        })?;

    // -- per-agent model/driver overrides ---------------------------------------

    let agent_names: BTreeSet<&str> = all_settings
        .keys()
        .filter_map(|k| k.strip_prefix("llm.agent_overrides."))
        .filter_map(|k| k.split('.').next())
        .collect();

    for &agent in &agent_names {
        let model_key = format!("llm.agent_overrides.{agent}.model");
        let driver_key = format!("llm.agent_overrides.{agent}.provider");

        let model = all_settings
            .get(&model_key)
            .filter(|v| !v.trim().is_empty())
            .cloned();
        let driver = all_settings
            .get(&driver_key)
            .filter(|v| !v.trim().is_empty())
            .cloned();

        if model.is_some() || driver.is_some() {
            info!(agent, ?model, ?driver, "per-agent LLM override");
            registry.set_agent_override(
                agent,
                rara_kernel::llm::registry::AgentDriverConfig { driver, model },
            );
        }
    }

    // -- codex (OpenAI via OAuth) — special-cased -----------------------------

    match rara_codex_oauth::load_tokens(credential_store).await {
        Ok(Some(tokens)) => {
            registry.register_driver(
                "codex",
                Arc::new(OpenAiDriver::new(
                    "https://api.openai.com/v1",
                    tokens.access_token,
                )),
            );
        }
        Ok(None) => {} // No tokens configured — skip
        Err(e) => tracing::warn!("failed to load codex OAuth tokens: {e}"),
    }

    info!(
        providers = ?provider_names,
        "driver registry: default_driver={default_provider}, default_model={default_model}",
    );
    Ok(registry)
}

// =========================================================================
// Private: InMemoryUserStore
// =========================================================================

use std::collections::HashMap;

use rara_kernel::{
    error::Result as KernelResult,
    identity::{KernelUser, Permission, Role, UserStore},
};

fn parse_role(s: &str) -> Role {
    match s {
        "root" => Role::Root,
        "admin" => Role::Admin,
        _ => Role::User,
    }
}

fn default_permissions(role: Role) -> Vec<Permission> {
    match role {
        Role::Root | Role::Admin => vec![Permission::All],
        Role::User => vec![],
    }
}

/// In-memory user store built from YAML config at startup.
struct InMemoryUserStore {
    by_name: HashMap<String, KernelUser>,
}

impl InMemoryUserStore {
    fn from_config(users: &[UserConfig]) -> Self {
        let by_name: HashMap<String, KernelUser> = users
            .iter()
            .map(|u| {
                let role = parse_role(&u.role);
                let perms = default_permissions(role);
                (
                    u.name.clone(),
                    KernelUser {
                        name: u.name.clone(),
                        role,
                        permissions: perms,
                        enabled: true,
                    },
                )
            })
            .collect();

        let mut store = Self { by_name };

        // Ensure a built-in "system" user exists so internal subsystems
        // (e.g. Mita heartbeat) can resolve a principal without requiring
        // explicit YAML configuration.
        store
            .by_name
            .entry("system".to_string())
            .or_insert_with(|| KernelUser {
                name:        "system".to_string(),
                role:        Role::Root,
                permissions: default_permissions(Role::Root),
                enabled:     true,
            });

        store
    }
}

#[async_trait]
impl UserStore for InMemoryUserStore {
    async fn get_by_name(&self, name: &str) -> KernelResult<Option<KernelUser>> {
        Ok(self.by_name.get(name).cloned())
    }

    async fn list(&self) -> KernelResult<Vec<KernelUser>> {
        Ok(self.by_name.values().cloned().collect())
    }
}

// =========================================================================
// Private: PlatformIdentityResolver
// =========================================================================

use rara_kernel::{
    channel::types::ChannelType,
    identity::UserId,
    io::{IOError, IdentityResolver},
};
use tracing::debug;

/// Config-driven identity resolver that maps platform identities to kernel
/// users via an in-memory lookup table built from YAML configuration.
struct PlatformIdentityResolver {
    /// `(channel_type, platform_uid)` -> kernel user name.
    mappings: HashMap<(String, String), String>,
}

impl PlatformIdentityResolver {
    /// Build the resolver from the configured user list.
    fn new(users: &[UserConfig]) -> Self {
        let mut mappings = HashMap::new();
        for u in users {
            mappings.insert(("cli".to_string(), u.name.clone()), u.name.clone());
            mappings.insert(
                ("cli".to_string(), format!("cli:{}", u.name)),
                u.name.clone(),
            );
            for p in &u.platforms {
                mappings.insert(
                    (p.channel_type.to_lowercase(), p.user_id.clone()),
                    u.name.clone(),
                );
            }
        }
        Self { mappings }
    }
}

#[async_trait]
impl IdentityResolver for PlatformIdentityResolver {
    async fn resolve(
        &self,
        channel_type: ChannelType,
        platform_user_id: &str,
        _platform_chat_id: Option<&str>,
    ) -> std::result::Result<UserId, IOError> {
        let key = (
            channel_type.to_string().to_lowercase(),
            platform_user_id.to_string(),
        );
        debug!(channel = %channel_type, platform_user_id, "resolving identity");
        self.mappings.get(&key).cloned().map(UserId).ok_or_else(|| {
            IOError::IdentityResolutionFailed {
                message: format!("unknown platform user: {platform_user_id}"),
            }
        })
    }
}

// =========================================================================
// Private: Agent manifests
// =========================================================================

/// Load agent manifests and build an AgentRegistry.
fn load_default_registry() -> rara_kernel::agent::AgentRegistry {
    use rara_kernel::agent::{AgentRegistry, ManifestLoader};

    let mut rara_manifest = rara_agents::rara().clone();
    rara_manifest.tools = crate::tools::rara_tool_names();

    let builtin = vec![
        (rara_manifest.clone(), Role::Root),
        (rara_manifest, Role::Admin),
        (rara_agents::nana().clone(), Role::User),
        (rara_agents::worker().clone(), Role::User),
        (rara_agents::mita().clone(), Role::Root),
    ];
    let agents_dir = rara_paths::data_dir().join("agents");
    let mut loader = ManifestLoader::new();
    let _ = loader.load_dir(&agents_dir);
    let registry = AgentRegistry::init(builtin, &loader, agents_dir);
    info!(count = registry.list().len(), "agent registry initialized");
    registry
}

// =========================================================================
// Private: MCP
// =========================================================================

/// Initialize the MCP manager from the filesystem registry and start all
/// enabled servers.
async fn init_mcp_manager(
    credential_store: rara_keyring_store::KeyringStoreRef,
) -> std::result::Result<rara_mcp::manager::mgr::McpManager, Whatever> {
    use rara_mcp::{
        manager::{mgr::McpManager, registry::FSMcpRegistry},
        oauth::OAuthCredentialsStoreMode,
    };

    let path = rara_paths::config_dir().join("mcp-servers.json");
    let registry = FSMcpRegistry::load(&path)
        .await
        .whatever_context("failed to load MCP registry")?;
    ensure_context_mode_builtin(&registry).await;

    let manager = McpManager::new(
        Arc::new(registry),
        OAuthCredentialsStoreMode::default(),
        credential_store,
    );
    let started = manager.start_enabled().await;
    if started.is_empty() {
        info!("no MCP servers to start");
    } else {
        info!(servers = ?started, "MCP servers started");
    }
    Ok(manager)
}

/// Register context-mode as a builtin MCP server if not already present,
/// or re-enable it if it was somehow disabled.
async fn ensure_context_mode_builtin(registry: &rara_mcp::manager::registry::FSMcpRegistry) {
    use rara_mcp::manager::registry::{McpRegistry, McpServerConfig};

    const NAME: &str = "context-mode";

    if let Ok(Some(existing)) = registry.get(NAME).await {
        // Already registered. Ensure it is enabled and marked builtin.
        if !existing.enabled || !existing.builtin {
            let mut fixed = existing;
            fixed.enabled = true;
            fixed.builtin = true;
            if let Err(e) = registry.add(NAME.to_string(), fixed).await {
                tracing::warn!(error = %e, "failed to fix context-mode config");
            }
        }
        return;
    }

    let config = McpServerConfig {
        command: "npx".to_string(),
        args: vec!["-y".to_string(), "context-mode@latest".to_string()],
        enabled: true,
        builtin: true,
        ..Default::default()
    };

    if let Err(e) = registry.add(NAME.to_string(), config).await {
        tracing::warn!(error = %e, "failed to register builtin context-mode MCP server");
    } else {
        info!("registered builtin context-mode MCP server");
    }
}

// =========================================================================
// ACP agent registry
// =========================================================================

/// Initialize the ACP agent registry from disk and ensure built-in agents
/// are present.
async fn init_acp_registry() -> std::result::Result<rara_acp::AcpRegistryRef, Whatever> {
    use rara_acp::registry::FSAcpRegistry;

    let path = rara_paths::config_dir().join("acp-agents.json");
    let registry = FSAcpRegistry::load(&path)
        .await
        .whatever_context("failed to load ACP registry")?;
    Ok(Arc::new(registry))
}

// =========================================================================
// McpDynamicToolProvider
// =========================================================================

/// Bridges [`McpManager`](rara_mcp::manager::mgr::McpManager) into the
/// kernel's [`rara_kernel::tool::DynamicToolProvider`] trait so that MCP tools
/// are injected into every `GetToolRegistry` syscall at runtime.
pub(crate) struct McpDynamicToolProvider {
    manager: rara_mcp::manager::mgr::McpManager,
}

impl McpDynamicToolProvider {
    pub fn new(manager: rara_mcp::manager::mgr::McpManager) -> Self { Self { manager } }
}

#[async_trait]
impl rara_kernel::tool::DynamicToolProvider for McpDynamicToolProvider {
    async fn tools(&self) -> Vec<rara_kernel::tool::AgentToolRef> {
        match rara_mcp::tool_bridge::McpToolBridge::from_manager(self.manager.clone()).await {
            Ok(bridges) => bridges
                .into_iter()
                .map(|b| Arc::new(b) as rara_kernel::tool::AgentToolRef)
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "failed to load MCP dynamic tools");
                Vec::new()
            }
        }
    }
}

// =========================================================================
// Private: Composio auth provider
// =========================================================================

/// Composio auth provider that reads credentials from runtime settings.
#[derive(Clone)]
struct SettingsComposioAuthProvider {
    settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
}

impl SettingsComposioAuthProvider {
    fn new(settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>) -> Self {
        Self { settings }
    }
}

#[async_trait]
impl rara_composio::ComposioAuthProvider for SettingsComposioAuthProvider {
    async fn acquire_auth(&self) -> anyhow::Result<rara_composio::ComposioAuth> {
        use rara_domain_shared::settings::keys;
        let api_key = self
            .settings
            .get(keys::COMPOSIO_API_KEY)
            .await
            .ok_or_else(|| anyhow::anyhow!("composio.api_key is not configured in settings"))?;
        let entity_id = self.settings.get(keys::COMPOSIO_ENTITY_ID).await;
        Ok(rara_composio::ComposioAuth::new(
            api_key,
            entity_id.as_deref(),
        ))
    }
}

// =========================================================================
// Private: Knowledge Layer initialization
// =========================================================================

/// Initialize the knowledge layer — all configuration read from settings,
/// reuses the application's shared SQLite pool.
async fn init_knowledge_service(
    pool: sqlx::SqlitePool,
    settings: &dyn rara_domain_shared::settings::SettingsProvider,
    embedder: rara_kernel::llm::LlmEmbedderRef,
) -> anyhow::Result<rara_kernel::memory::knowledge::KnowledgeServiceRef> {
    use rara_domain_shared::settings::keys;
    use rara_kernel::memory::knowledge::{EmbeddingService, KnowledgeConfig, KnowledgeService};

    let embedding_model = settings
        .get(keys::KNOWLEDGE_EMBEDDING_MODEL)
        .await
        .ok_or_else(|| anyhow::anyhow!("{} is not configured", keys::KNOWLEDGE_EMBEDDING_MODEL))?;
    let embedding_dimensions: usize = settings
        .get(keys::KNOWLEDGE_EMBEDDING_DIMENSIONS)
        .await
        .ok_or_else(|| {
            anyhow::anyhow!("{} is not configured", keys::KNOWLEDGE_EMBEDDING_DIMENSIONS)
        })?
        .parse()?;
    let search_top_k: usize = settings
        .get(keys::KNOWLEDGE_SEARCH_TOP_K)
        .await
        .ok_or_else(|| anyhow::anyhow!("{} is not configured", keys::KNOWLEDGE_SEARCH_TOP_K))?
        .parse()?;
    let similarity_threshold: f32 = settings
        .get(keys::KNOWLEDGE_SIMILARITY_THRESHOLD)
        .await
        .ok_or_else(|| {
            anyhow::anyhow!("{} is not configured", keys::KNOWLEDGE_SIMILARITY_THRESHOLD)
        })?
        .parse()?;
    let extractor_model = settings
        .get(keys::KNOWLEDGE_EXTRACTOR_MODEL)
        .await
        .ok_or_else(|| anyhow::anyhow!("{} is not configured", keys::KNOWLEDGE_EXTRACTOR_MODEL))?;

    let config = KnowledgeConfig::builder()
        .embedding_dimensions(embedding_dimensions)
        .search_top_k(search_top_k)
        .similarity_threshold(similarity_threshold)
        .build();

    let embedding_svc = Arc::new(EmbeddingService::new(
        config.clone(),
        embedder,
        embedding_model,
    )?);

    info!("knowledge layer initialized");
    Ok(Arc::new(KnowledgeService {
        pool,
        embedding_svc,
        config,
        extractor_model,
    }))
}
