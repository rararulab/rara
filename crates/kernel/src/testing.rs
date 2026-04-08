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
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

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
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("in-memory SQLite pool");

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
        pool,
        embedding_svc: Arc::new(embedding_svc),
        config,
        extractor_model: "scripted".to_string(),
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

    /// Set the scripted LLM responses.
    #[must_use]
    pub fn responses(mut self, responses: Vec<CompletionResponse>) -> Self {
        self.responses = responses;
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

        // LLM driver
        let driver: LlmDriverRef = Arc::new(ScriptedLlmDriver::new(self.responses));
        let driver_registry = Arc::new(DriverRegistry::new("scripted"));
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

        // Trace service (in-memory SQLite)
        let trace_pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory SQLite for traces");
        let trace_service = crate::trace::TraceService::new(trace_pool);

        // Skills prompt (empty)
        let skill_prompt_provider: crate::handle::SkillPromptProvider = Arc::new(|| String::new());

        let kernel = Kernel::new(
            self.config,
            driver_registry,
            tool_registry,
            agent_registry,
            session_index,
            tape_service,
            settings,
            security,
            io,
            knowledge,
            None, // no dynamic tool provider
            trace_service,
            skill_prompt_provider,
        );

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
    responses:   Mutex<VecDeque<serde_json::Value>>,
    captured:    Mutex<Vec<serde_json::Value>>,
}

impl FakeTool {
    /// Create a `FakeTool` with the given name and scripted responses.
    ///
    /// Each call to `execute` pops one response from the front of the queue.
    pub fn new(name: impl Into<String>, responses: Vec<serde_json::Value>) -> Self {
        Self {
            name:        name.into(),
            description: "Test tool (FakeTool)".to_string(),
            responses:   Mutex::new(VecDeque::from(responses)),
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
        let output = self
            .responses
            .lock()
            .expect("FakeTool responses mutex poisoned")
            .pop_front()
            .expect("FakeTool: no more scripted responses");
        Ok(ToolOutput::from(output))
    }
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
