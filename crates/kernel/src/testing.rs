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

//! Test infrastructure for booting a minimal kernel without real services.
//!
//! Provides [`TestKernelBuilder`] for constructing a [`Kernel`] with
//! scripted LLM responses, in-memory session index, temp-dir tape storage,
//! and no external dependencies (no DB, no network, no API keys).

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Once, OnceLock},
};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use yunara_store::diesel_pool::{DieselPoolConfig, DieselSqlitePools, build_sqlite_pools};

use crate::{
    agent::{AgentManifest, AgentRegistry, AgentRole, ManifestLoader},
    handle::KernelHandle,
    identity::{KernelUser, Permission, Role, UserStoreRef},
    io::IOSubsystem,
    kernel::{Kernel, KernelConfig},
    llm::{CompletionResponse, DriverRegistry, LlmDriverRef, ScriptedLlmDriver, StopReason},
    memory::{FileTapeStore, TapeService},
    security::{ApprovalManager, ApprovalPolicy, SecuritySubsystem},
    session::test_utils::InMemorySessionIndex,
    tool::{AgentTool, AgentToolRef, ToolContext, ToolOutput, ToolRegistry},
};

/// Build an in-memory-ish diesel-async SQLite pool pre-populated with the
/// schema the kernel's DB-backed services touch (tape FTS, traces, memory
/// items).
///
/// Uses a per-pool unique temp file rather than `:memory:` because diesel's
/// SQLite URL handling opens literal paths (so `:memory:` collides across
/// parallel tests) and because `bb8` may open multiple connections that must
/// see the same database — shared in-memory SQLite across connections
/// requires URI filename tricks we don't need here. A temp file is deleted
/// when the pool drops in practice (OS cleanup on test process exit).
pub async fn build_memory_diesel_pools() -> DieselSqlitePools {
    use diesel_async::RunQueryDsl as _;
    let db_path = std::env::temp_dir().join(format!("rara-test-{}.sqlite", uuid::Uuid::new_v4()));
    let pools = build_sqlite_pools(
        &DieselPoolConfig::builder()
            .database_url(db_path.to_string_lossy().into_owned())
            .max_connections(1)
            .build(),
    )
    .await
    .expect("in-memory diesel pool");
    let mut conn = pools.writer.get().await.expect("pool conn");
    for ddl in MEMORY_TEST_SCHEMA {
        diesel::sql_query(*ddl)
            .execute(&mut *conn)
            .await
            .expect("bootstrap schema");
    }
    drop(conn);
    pools
}

/// DDL the in-memory test pool installs on boot. Mirrors the production
/// migrations needed by the kernel's test surface (tape FTS, traces,
/// memory items). Kept inline because `diesel_migrations::embed_migrations!`
/// has not landed yet (see #1702 cutover step).
const MEMORY_TEST_SCHEMA: &[&str] = &[
    "CREATE VIRTUAL TABLE tape_fts USING fts5(content, tape_name UNINDEXED, entry_kind UNINDEXED, \
     entry_id UNINDEXED, session_key UNINDEXED, tokenize = 'unicode61 remove_diacritics 2')",
    "CREATE TABLE tape_fts_meta (tape_name TEXT PRIMARY KEY, last_indexed_id INTEGER NOT NULL \
     DEFAULT 0)",
    "CREATE TABLE execution_traces (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, trace_data \
     TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
    "CREATE TABLE memory_items (id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT NOT NULL, \
     content TEXT NOT NULL, memory_type TEXT NOT NULL, category TEXT NOT NULL, source_tape TEXT, \
     source_entry_id INTEGER, embedding BLOB, created_at TEXT NOT NULL DEFAULT (datetime('now')), \
     updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
];

// ---------------------------------------------------------------------------
// Stub SettingsProvider
// ---------------------------------------------------------------------------

/// Minimal settings provider backed by an in-memory `HashMap`.
struct StubSettings {
    data: tokio::sync::RwLock<HashMap<String, String>>,
    tx:   tokio::sync::watch::Sender<()>,
    rx:   tokio::sync::watch::Receiver<()>,
}

impl StubSettings {
    fn new() -> Self {
        let (tx, rx) = tokio::sync::watch::channel(());
        Self {
            data: tokio::sync::RwLock::new(HashMap::new()),
            tx,
            rx,
        }
    }
}

#[async_trait]
impl rara_domain_shared::settings::SettingsProvider for StubSettings {
    async fn get(&self, key: &str) -> Option<String> { self.data.read().await.get(key).cloned() }

    async fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.data
            .write()
            .await
            .insert(key.to_string(), value.to_string());
        let _ = self.tx.send(());
        Ok(())
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.data.write().await.remove(key);
        let _ = self.tx.send(());
        Ok(())
    }

    async fn list(&self) -> HashMap<String, String> { self.data.read().await.clone() }

    async fn batch_update(&self, patches: HashMap<String, Option<String>>) -> anyhow::Result<()> {
        let mut data = self.data.write().await;
        for (key, value) in patches {
            match value {
                Some(v) => {
                    data.insert(key, v);
                }
                None => {
                    data.remove(&key);
                }
            }
        }
        let _ = self.tx.send(());
        Ok(())
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<()> { self.rx.clone() }
}

// ---------------------------------------------------------------------------
// Stub UserStore
// ---------------------------------------------------------------------------

/// Minimal user store with a single root user named `"test"`.
struct StubUserStore {
    users: Vec<KernelUser>,
}

impl StubUserStore {
    fn new() -> Self {
        Self {
            users: vec![KernelUser {
                name:        "test".into(),
                role:        Role::Root,
                permissions: vec![Permission::All],
                enabled:     true,
            }],
        }
    }
}

#[async_trait]
impl crate::identity::UserStore for StubUserStore {
    async fn get_by_name(&self, name: &str) -> crate::error::Result<Option<KernelUser>> {
        Ok(self.users.iter().find(|u| u.name == name).cloned())
    }

    async fn list(&self) -> crate::error::Result<Vec<KernelUser>> { Ok(self.users.clone()) }
}

// ---------------------------------------------------------------------------
// Stub IdentityResolver
// ---------------------------------------------------------------------------

/// Identity resolver that maps every platform user to `"test"`.
struct StubIdentityResolver;

#[async_trait]
impl crate::io::IdentityResolver for StubIdentityResolver {
    async fn resolve(
        &self,
        _channel_type: crate::channel::types::ChannelType,
        _platform_user_id: &str,
        _platform_chat_id: Option<&str>,
    ) -> Result<crate::identity::UserId, crate::io::IOError> {
        Ok(crate::identity::UserId("test".to_string()))
    }
}

// ---------------------------------------------------------------------------
// Stub KnowledgeService
// ---------------------------------------------------------------------------

/// Build a minimal knowledge service backed by in-memory SQLite and a noop
/// embedder.
async fn stub_knowledge_service() -> crate::memory::knowledge::KnowledgeServiceRef {
    let pool = build_memory_diesel_pools().await;

    let config = crate::memory::knowledge::KnowledgeConfig::builder()
        .embedding_dimensions(64_usize)
        .search_top_k(5_usize)
        .similarity_threshold(0.8_f32)
        .build();

    let embedder: crate::llm::LlmEmbedderRef = Arc::new(NoopEmbedder);
    let index_path = std::env::temp_dir()
        .join(format!("rara-test-{}", uuid::Uuid::new_v4()))
        .join("memory.usearch");
    let embedding_svc = crate::memory::knowledge::EmbeddingService::with_path(
        config.clone(),
        embedder,
        "noop".to_string(),
        index_path,
    )
    .expect("noop embedding service");

    Arc::new(crate::memory::knowledge::KnowledgeService {
        pools: pool,
        embedding_svc: Arc::new(embedding_svc),
        config,
    })
}

/// Embedder that returns zero vectors.
struct NoopEmbedder;

#[async_trait]
impl crate::llm::LlmEmbedder for NoopEmbedder {
    async fn embed(
        &self,
        request: crate::llm::EmbeddingRequest,
    ) -> crate::error::Result<crate::llm::EmbeddingResponse> {
        let embeddings = request.input.iter().map(|_| vec![0.0_f32; 64]).collect();
        Ok(crate::llm::EmbeddingResponse::builder()
            .embeddings(embeddings)
            .model("noop".to_string())
            .build())
    }
}

// ---------------------------------------------------------------------------
// TestKernelBuilder
// ---------------------------------------------------------------------------

/// Builder for a minimal test kernel.
///
/// Provides sane defaults for all subsystems so callers only need to
/// configure the parts they care about (typically the LLM responses).
///
/// # Example
///
/// ```ignore
/// let built = TestKernelBuilder::new(tmp_dir)
///     .responses(vec![scripted_response("Hi!")])
///     .manifest(my_manifest)
///     .build()
///     .await;
/// let handle = built.handle;
/// ```
pub struct TestKernelBuilder {
    tmp_dir:   PathBuf,
    responses: Vec<CompletionResponse>,
    results:   Option<Vec<crate::error::Result<CompletionResponse>>>,
    manifest:  Option<AgentManifest>,
    config:    KernelConfig,
    tools:     Vec<AgentToolRef>,
}

impl TestKernelBuilder {
    /// Create a builder rooted at the given temp directory.
    ///
    /// The directory is used for tape storage and agent manifest persistence.
    /// Context folding and Mita heartbeat are disabled by default to avoid
    /// unexpected LLM calls in tests.
    pub fn new(tmp_dir: &Path) -> Self {
        let mut config = KernelConfig::default();
        // Disable context folding -- it triggers extra LLM calls for
        // summarization that would exhaust scripted responses.
        config.context_folding.enabled = false;
        // Disable Mita heartbeat (already None by default, but be explicit).
        config.mita_heartbeat_interval = None;
        Self {
            tmp_dir: tmp_dir.to_path_buf(),
            responses: Vec::new(),
            results: None,
            manifest: None,
            config,
            tools: Vec::new(),
        }
    }

    /// Register a tool with the test kernel's tool registry.
    ///
    /// Tools are registered before the kernel starts, so they are visible to
    /// the agent runtime from the first turn.
    #[must_use]
    pub fn with_tool(mut self, tool: AgentToolRef) -> Self {
        self.tools.push(tool);
        self
    }

    /// Set the scripted LLM responses (all succeed).
    #[must_use]
    pub fn responses(mut self, responses: Vec<CompletionResponse>) -> Self {
        self.responses = responses;
        self
    }

    /// Set scripted LLM results that may include errors.
    ///
    /// When set, takes precedence over [`responses`](Self::responses).
    /// Use this to exercise the kernel's LLM error recovery paths.
    #[must_use]
    pub fn with_results(mut self, results: Vec<crate::error::Result<CompletionResponse>>) -> Self {
        self.results = Some(results);
        self
    }

    /// Override the default agent manifest.
    #[must_use]
    pub fn manifest(mut self, manifest: AgentManifest) -> Self {
        self.manifest = Some(manifest);
        self
    }

    /// Override the kernel config.
    #[must_use]
    pub fn config(mut self, config: KernelConfig) -> Self {
        self.config = config;
        self
    }

    /// Build and start the kernel, returning a [`TestKernel`] handle.
    pub async fn build(self) -> TestKernel {
        let tape_dir = self.tmp_dir.join("tapes");
        let agents_dir = self.tmp_dir.join("agents");
        std::fs::create_dir_all(&tape_dir).expect("create tape dir");
        std::fs::create_dir_all(&agents_dir).expect("create agents dir");

        // LLM driver — use pre-built Result queue when provided, otherwise
        // wrap all responses in Ok.
        let driver: LlmDriverRef = if let Some(results) = self.results {
            Arc::new(ScriptedLlmDriver::new_with_results(results))
        } else {
            Arc::new(ScriptedLlmDriver::new(self.responses))
        };
        let driver_registry = Arc::new(DriverRegistry::new(
            "scripted",
            Arc::new(crate::llm::OpenRouterCatalog::new()),
        ));
        driver_registry.register_driver("scripted", driver);
        driver_registry.set_provider_model("scripted", "scripted-model", vec![]);

        // Tool registry -- seed with any tools registered on the builder.
        let mut registry = ToolRegistry::new();
        for tool in self.tools {
            registry.register(tool);
        }
        let tool_registry = Arc::new(registry);

        // Agent manifest
        let manifest = self.manifest.unwrap_or_else(|| AgentManifest {
            name:                   "test-agent".to_string(),
            role:                   AgentRole::Chat,
            description:            "Test agent".to_string(),
            model:                  Some("scripted-model".to_string()),
            system_prompt:          "You are a test agent.".to_string(),
            soul_prompt:            None,
            provider_hint:          Some("scripted".to_string()),
            max_iterations:         Some(3),
            tools:                  Vec::new(),
            excluded_tools:         Vec::new(),
            max_children:           None,
            max_context_tokens:     None,
            priority:               Default::default(),
            metadata:               serde_json::Value::Null,
            sandbox:                None,
            default_execution_mode: None,
            tool_call_limit:        None,
            worker_timeout_secs:    None,
            max_continuations:      None,
            max_output_chars:       None,
        });
        let manifest_name = manifest.name.clone();

        let loader = ManifestLoader::new();
        let agent_registry = Arc::new(AgentRegistry::init(
            vec![(manifest, Role::Root)],
            &loader,
            agents_dir,
        ));

        // Session index
        let session_index: Arc<dyn crate::session::SessionIndex> =
            Arc::new(InMemorySessionIndex::new());

        // Tape service
        let tape_store = FileTapeStore::new(&tape_dir, &self.tmp_dir)
            .await
            .expect("test tape store");
        let tape_service = TapeService::new(tape_store);

        // Settings
        let settings: crate::kernel::SettingsRef = Arc::new(StubSettings::new());

        // Security
        let user_store: UserStoreRef = Arc::new(StubUserStore::new());
        let security = Arc::new(SecuritySubsystem::new(
            user_store,
            Arc::new(ApprovalManager::new(ApprovalPolicy::default())),
        ));

        // IO subsystem (no adapters)
        let identity_resolver: Arc<dyn crate::io::IdentityResolver> =
            Arc::new(StubIdentityResolver);
        let io = IOSubsystem::new(identity_resolver, session_index.clone(), None, 100);

        // Knowledge service
        let knowledge = stub_knowledge_service().await;

        // Trace service (in-memory diesel SQLite)
        let trace_pool = build_memory_diesel_pools().await;
        let trace_service = crate::trace::TraceService::new(trace_pool);

        // Skills prompt (empty)
        let skill_prompt_provider: crate::handle::SkillPromptProvider = Arc::new(|| String::new());

        // Redirect `rara_paths` config + data globals to a per-process
        // tempdir before any kernel code resolves them. Without this, the
        // agent loop's `build_system_prompt` calls `rara_paths::workspace_dir()`
        // which falls back to `$HOME/.config/rara/workspace`; on the
        // `arc-runner-set` CI runner `$HOME` is read-only and the resulting
        // `mkdir_all` panics with EACCES (issue #1989). Mirrors the pattern
        // used by `crates/channels/tests/web_session_smoke.rs`.
        redirect_rara_paths_for_tests();

        // Per-test scheduler dir — isolates `jobs.json` / `in_flight.json` /
        // `subscriptions.json` from every other test running in the same
        // process. `rara_paths::workspace_dir()` is a `OnceLock` global, so
        // without this redirect parallel tests share one `jobs.json` and
        // see each other's seeded jobs (observed as flaky
        // `list_on_empty_kernel_is_empty`).
        let scheduler_dir = self.tmp_dir.join("scheduler");
        let kernel = Kernel::builder()
            .config(self.config)
            .driver_registry(driver_registry)
            .tool_registry(tool_registry)
            .agent_registry(agent_registry)
            .session_index(session_index)
            .tape_service(tape_service)
            .settings(settings)
            .security(security)
            .io(io)
            .knowledge(knowledge)
            .trace_service(trace_service)
            .skill_prompt_provider(skill_prompt_provider)
            .scheduler_dir(scheduler_dir)
            .build();

        let cancel_token = CancellationToken::new();
        let (_kernel_arc, handle) = kernel.start(cancel_token.clone());

        TestKernel {
            handle,
            cancel_token,
            agent_name: manifest_name,
        }
    }
}

/// A running test kernel with its handle and cancellation token.
pub struct TestKernel {
    /// Kernel handle for interacting with the running kernel.
    pub handle:       KernelHandle,
    /// Cancellation token for shutting down the kernel.
    pub cancel_token: CancellationToken,
    /// Name of the default agent manifest.
    pub agent_name:   String,
}

impl TestKernel {
    /// Shut down the kernel gracefully.
    pub fn shutdown(&self) { self.cancel_token.cancel(); }

    /// Seed a pre-built [`JobEntry`](crate::schedule::JobEntry) directly onto
    /// the wheel, bypassing the `RegisterJob` syscall path.
    ///
    /// The syscall path requires a real session principal in the process
    /// table, which integration tests typically don't set up. This helper is
    /// the only public surface for wheel-seeding; the underlying
    /// `KernelHandle::__seed_job_unsafe_test_harness` is crate-private and
    /// deliberately named to warn off in-crate callers.
    pub fn seed_job(&self, entry: crate::schedule::JobEntry) {
        self.handle.__seed_job_unsafe_test_harness(entry);
    }
}

/// Convenience helper: build a [`CompletionResponse`] with text content.
pub fn scripted_response(text: &str) -> CompletionResponse {
    CompletionResponse {
        content:           Some(text.to_string()),
        reasoning_content: None,
        tool_calls:        vec![],
        stop_reason:       StopReason::Stop,
        usage:             None,
        model:             "scripted".to_string(),
    }
}

// ---------------------------------------------------------------------------
// FakeTool
// ---------------------------------------------------------------------------

/// Test-only [`AgentTool`] that returns pre-recorded outputs for each call
/// and captures received inputs for post-hoc assertions.
///
/// Unlike real tools, `FakeTool` performs no I/O: every invocation pops the
/// next queued response and records the input. This makes it a drop-in
/// scripted counterpart to [`ScriptedLlmDriver`] for tool-call round-trip
/// tests.
///
/// # Panics
///
/// `execute` panics if called more times than there are scripted responses.
pub struct FakeTool {
    name:        String,
    description: String,
    responses:   Mutex<VecDeque<std::result::Result<serde_json::Value, String>>>,
    captured:    Mutex<Vec<serde_json::Value>>,
}

impl FakeTool {
    /// Create a `FakeTool` with the given name and scripted success responses.
    ///
    /// Each call to `execute` pops one response from the front of the queue
    /// and returns it as `Ok`. For failure-mode tests that need the tool to
    /// return errors, use [`Self::with_results`] instead.
    pub fn new(name: impl Into<String>, responses: Vec<serde_json::Value>) -> Self {
        Self {
            name:        name.into(),
            description: "Test tool (FakeTool)".to_string(),
            responses:   Mutex::new(responses.into_iter().map(Ok).collect()),
            captured:    Mutex::new(Vec::new()),
        }
    }

    /// Create a `FakeTool` whose scripted queue may contain errors.
    ///
    /// An `Err(message)` entry causes the matching `execute` call to surface a
    /// tool failure with the given message — the canonical way to exercise
    /// the kernel's tool-error feedback path.
    pub fn with_results(
        name: impl Into<String>,
        results: Vec<std::result::Result<serde_json::Value, String>>,
    ) -> Self {
        Self {
            name:        name.into(),
            description: "Test tool (FakeTool)".to_string(),
            responses:   Mutex::new(VecDeque::from(results)),
            captured:    Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of every input the tool has received so far.
    #[must_use]
    pub fn captured_inputs(&self) -> Vec<serde_json::Value> {
        self.captured
            .lock()
            .expect("FakeTool captured mutex poisoned")
            .clone()
    }
}

#[async_trait]
impl AgentTool for FakeTool {
    fn name(&self) -> &str { &self.name }

    fn description(&self) -> &str { &self.description }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": true,
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        self.captured
            .lock()
            .expect("FakeTool captured mutex poisoned")
            .push(params);
        let entry = self
            .responses
            .lock()
            .expect("FakeTool responses mutex poisoned")
            .pop_front()
            .expect("FakeTool: no more scripted responses");
        match entry {
            Ok(value) => Ok(ToolOutput::from(value)),
            Err(message) => Err(anyhow::anyhow!(message)),
        }
    }
}

// ---------------------------------------------------------------------------
// ValidatingFakeTool
// ---------------------------------------------------------------------------

/// A [`FakeTool`] variant whose [`validate`](AgentTool::validate) rejects
/// inputs containing a `"reject"` key set to `true`.
///
/// Used to test that the agent loop correctly short-circuits on validation
/// failures without calling `execute`.
pub struct ValidatingFakeTool {
    inner: FakeTool,
}

impl ValidatingFakeTool {
    /// Create a `ValidatingFakeTool` with the given name and scripted
    /// responses.
    pub fn new(name: impl Into<String>, responses: Vec<serde_json::Value>) -> Self {
        Self {
            inner: FakeTool::new(name, responses),
        }
    }

    /// Return captured inputs that made it through validation to `execute`.
    pub fn captured_inputs(&self) -> Vec<serde_json::Value> { self.inner.captured_inputs() }
}

#[async_trait]
impl AgentTool for ValidatingFakeTool {
    fn name(&self) -> &str { self.inner.name() }

    fn description(&self) -> &str { self.inner.description() }

    fn parameters_schema(&self) -> serde_json::Value { self.inner.parameters_schema() }

    async fn validate(&self, params: &serde_json::Value) -> anyhow::Result<()> {
        if params
            .get("reject")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            anyhow::bail!("validation rejected: 'reject' flag is set");
        }
        Ok(())
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        self.inner.execute(params, context).await
    }
}

/// Redirect `rara_paths` config and data directories to a per-process
/// tempdir, exactly once per test binary.
///
/// `rara_paths::set_custom_*_dir` panics on second invocation, so the calls
/// are guarded by `Once`. The tempdir is held in a `OnceLock` static so it
/// outlives every `TestKernelBuilder` constructed in the same process.
fn redirect_rara_paths_for_tests() {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    static INIT: Once = Once::new();
    let root = ROOT.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("rara-test-kernel-builder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create test kernel builder root");
        dir
    });
    INIT.call_once(|| {
        let config = root.join("rara_config");
        let data = root.join("rara_data");
        std::fs::create_dir_all(&config).expect("create test config dir");
        std::fs::create_dir_all(&data).expect("create test data dir");
        rara_paths::set_custom_config_dir(&config);
        rara_paths::set_custom_data_dir(&data);
    });
}

/// Convenience helper: build a [`CompletionResponse`] with tool calls.
pub fn scripted_tool_call_response(
    tool_calls: Vec<crate::llm::ToolCallRequest>,
) -> CompletionResponse {
    CompletionResponse {
        content: None,
        reasoning_content: None,
        tool_calls,
        stop_reason: StopReason::ToolCalls,
        usage: None,
        model: "scripted".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calling `redirect_rara_paths_for_tests` twice in the same binary must
    /// not panic — the second call is a no-op because `rara_paths`'s
    /// `set_custom_*` panic on re-init. After both calls,
    /// `rara_paths::config_dir()` / `data_dir()` resolve to the tempdir
    /// roots seeded by the first call.
    #[test]
    fn test_kernel_builder_redirect_is_idempotent() {
        redirect_rara_paths_for_tests();
        let config_first = rara_paths::config_dir().clone();
        let data_first = rara_paths::data_dir().clone();

        // Second call must not panic and must not move the resolved paths.
        redirect_rara_paths_for_tests();
        assert_eq!(rara_paths::config_dir(), &config_first);
        assert_eq!(rara_paths::data_dir(), &data_first);

        // The redirect must have escaped `$HOME/.config/rara`; the resolved
        // dirs live under `std::env::temp_dir()` per the helper.
        let tmp = std::env::temp_dir();
        assert!(
            config_first.starts_with(&tmp),
            "config dir {} should live under temp dir {}",
            config_first.display(),
            tmp.display()
        );
        assert!(
            data_first.starts_with(&tmp),
            "data dir {} should live under temp dir {}",
            data_first.display(),
            tmp.display()
        );
    }
}
