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

//! Kernel — the unified orchestrator for agent lifecycle and execution.
//!
//! The [`Kernel`] is the single entry point for all agent operations.
//! It holds 7 component slots and drives the LLM ↔ Tool execution loop.

use std::sync::Arc;

use jiff::Timestamp;
use tracing::info;
use uuid::Uuid;

use crate::{
    context::AgentContext,
    error::{KernelError, Result},
    event::{EventBus, KernelEvent},
    guard::{Guard, GuardContext},
    llm::{ChatMessage, ChatRequest, ChatResponse, ChatRole, LlmProvider, ToolDefinition},
    memory::Memory,
    prompt::PromptRepo,
    registry::{AgentEntry, AgentManifest, AgentRegistry, AgentState},
    session::SessionStore,
    tool::ToolRegistry,
};

/// Kernel configuration.
#[derive(Debug, Clone)]
pub struct KernelConfig {
    /// Default user ID for memory context.
    pub user_id: Uuid,
}

/// The unified agent orchestrator.
///
/// Holds 7 component slots + agent registry. Drives the LLM ↔ Tool
/// execution loop for any registered agent.
pub struct Kernel {
    // -- 7 Components --
    llm:      Arc<dyn LlmProvider>,
    tools:    Arc<ToolRegistry>,
    memory:   Arc<dyn Memory>,
    sessions: Arc<dyn SessionStore>,
    prompts:  Arc<dyn PromptRepo>,
    guard:    Arc<dyn Guard>,
    bus:      Arc<dyn EventBus>,

    // -- Runtime state --
    registry: AgentRegistry,
    config:   KernelConfig,
}

impl Kernel {
    /// Boot a new kernel with all 7 components.
    pub fn boot(
        config: KernelConfig,
        llm: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        memory: Arc<dyn Memory>,
        sessions: Arc<dyn SessionStore>,
        prompts: Arc<dyn PromptRepo>,
        guard: Arc<dyn Guard>,
        bus: Arc<dyn EventBus>,
    ) -> Self {
        info!("Booting kernel");
        Self {
            llm,
            tools,
            memory,
            sessions,
            prompts,
            guard,
            bus,
            registry: AgentRegistry::new(),
            config,
        }
    }

    // -- Agent lifecycle -------------------------------------------------------

    /// Register a new agent and return its ID.
    pub fn register_agent(&self, manifest: AgentManifest) -> Result<Uuid> {
        let id = self.registry.register(manifest)?;
        info!(agent_id = %id, "Agent registered");
        Ok(id)
    }

    /// Get an agent entry by ID.
    pub fn get_agent(&self, id: Uuid) -> Result<AgentEntry> {
        self.registry.get(id)
    }

    /// Find an agent by name.
    pub fn find_agent(&self, name: &str) -> Option<AgentEntry> {
        self.registry.find_by_name(name)
    }

    /// List all registered agents.
    pub fn list_agents(&self) -> Vec<AgentEntry> {
        self.registry.list()
    }

    /// Remove an agent from the registry.
    pub fn kill_agent(&self, id: Uuid) -> Result<AgentEntry> {
        let entry = self.registry.remove(id)?;
        info!(agent_id = %id, name = %entry.name, "Agent killed");
        Ok(entry)
    }

    // -- Message dispatch ------------------------------------------------------

    /// Send a message to an agent and run the full LLM ↔ Tool loop.
    pub async fn send(&self, agent_id: Uuid, user_text: String) -> Result<ChatResponse> {
        let entry = self.registry.get(agent_id)?;
        self.registry.set_state(agent_id, AgentState::Running)?;

        let ctx = self.create_context(&entry);

        // 1. Load history
        let history = ctx.sessions.load_history(ctx.session_id).await
            .map_err(|e| KernelError::Session { message: e.to_string() })?;

        // 2. Build messages: history + user message
        let user_msg = ChatMessage {
            role:         ChatRole::User,
            content:      Some(user_text.clone()),
            tool_calls:   Vec::new(),
            tool_call_id: None,
        };

        let mut messages = history;
        messages.push(user_msg.clone());

        // 3. Run the LLM ↔ Tool loop
        let response = self.run_loop(&ctx, messages).await;

        // 4. Save exchange on success
        if let Ok(ref resp) = response {
            let assistant_msg = ChatMessage {
                role:         ChatRole::Assistant,
                content:      resp.content.clone(),
                tool_calls:   Vec::new(),
                tool_call_id: None,
            };
            let _ = ctx.sessions.append(
                ctx.session_id,
                crate::session::Exchange {
                    user_message:      user_msg,
                    assistant_message: assistant_msg,
                },
            ).await;
        }

        self.registry.set_state(agent_id, AgentState::Idle)?;
        response
    }

    // -- Core execution loop ---------------------------------------------------

    async fn run_loop(
        &self,
        ctx: &AgentContext,
        mut messages: Vec<ChatMessage>,
    ) -> Result<ChatResponse> {
        let guard_ctx = GuardContext {
            agent_id:   ctx.agent_id,
            user_id:    ctx.user_id,
            session_id: ctx.session_id,
        };

        // Build tool definitions from the registry
        let tool_defs: Vec<ToolDefinition> = ctx.tools.iter().map(|(name, tool, _, _)| {
            ToolDefinition {
                name:        name.to_owned(),
                description: tool.description().to_owned(),
                parameters:  tool.parameters_schema(),
            }
        }).collect();

        for iteration in 0..ctx.max_iterations {
            let request = ChatRequest {
                model:         ctx.model.clone(),
                system_prompt: ctx.system_prompt.clone(),
                messages:      messages.clone(),
                tools:         if tool_defs.is_empty() { None } else { Some(tool_defs.clone()) },
                temperature:   None,
            };

            let response = ctx.llm.chat(request).await?;

            // No tool calls → terminal response
            if response.tool_calls.is_empty() {
                info!(iterations = iteration + 1, "Agent loop completed");
                return Ok(response);
            }

            // Append assistant message with tool calls
            messages.push(ChatMessage {
                role:         ChatRole::Assistant,
                content:      response.content.clone(),
                tool_calls:   response.tool_calls.iter().map(|tc| crate::llm::ToolCall {
                    id:        tc.id.clone(),
                    name:      tc.name.clone(),
                    arguments: tc.arguments.clone(),
                }).collect(),
                tool_call_id: None,
            });

            // Execute each tool call
            for call in &response.tool_calls {
                // Guard check
                let verdict = ctx.guard.check_tool(&guard_ctx, &call.name, &call.arguments).await;
                let tool_result = if verdict.is_allow() {
                    match ctx.tools.get(&call.name) {
                        Some(tool) => {
                            match tool.execute(call.arguments.clone()).await {
                                Ok(result) => {
                                    ctx.bus.publish(KernelEvent::ToolExecuted {
                                        agent_id:  ctx.agent_id,
                                        tool_name: call.name.to_string(),
                                        success:   true,
                                        timestamp: Timestamp::now(),
                                    }).await;
                                    result.to_string()
                                }
                                Err(e) => {
                                    ctx.bus.publish(KernelEvent::ToolExecuted {
                                        agent_id:  ctx.agent_id,
                                        tool_name: call.name.to_string(),
                                        success:   false,
                                        timestamp: Timestamp::now(),
                                    }).await;
                                    format!("Tool error: {e}")
                                }
                            }
                        }
                        None => format!("Tool not found: {}", call.name),
                    }
                } else {
                    let reason = match &verdict {
                        crate::guard::Verdict::Deny { reason } => reason.clone(),
                        crate::guard::Verdict::NeedApproval { prompt } => {
                            format!("Approval required: {prompt}")
                        }
                        _ => "Denied".to_owned(),
                    };
                    ctx.bus.publish(KernelEvent::GuardDenied {
                        agent_id:  ctx.agent_id,
                        tool_name: call.name.to_string(),
                        reason:    reason.clone(),
                        timestamp: Timestamp::now(),
                    }).await;
                    format!("Guard denied: {reason}")
                };

                // Append tool result message
                messages.push(ChatMessage {
                    role:         ChatRole::Tool,
                    content:      Some(tool_result),
                    tool_calls:   Vec::new(),
                    tool_call_id: Some(call.id.to_string()),
                });
            }
        }

        // Max iterations exceeded
        Err(KernelError::Llm {
            message: format!(
                "agent loop exceeded max iterations ({})",
                ctx.max_iterations
            ),
        })
    }

    // -- Context creation ------------------------------------------------------

    fn create_context(&self, entry: &AgentEntry) -> AgentContext {
        AgentContext {
            agent_id:       entry.id,
            session_id:     entry.session_id,
            user_id:        self.config.user_id,
            llm:            Arc::clone(&self.llm),
            tools:          Arc::new(self.tools.filtered(&entry.tools)),
            memory:         Arc::clone(&self.memory),
            sessions:       Arc::clone(&self.sessions),
            prompts:        Arc::clone(&self.prompts),
            guard:          Arc::clone(&self.guard),
            bus:            Arc::clone(&self.bus),
            model:          entry.model.clone(),
            system_prompt:  entry.system_prompt.clone(),
            max_iterations: entry.max_iterations,
        }
    }

    // -- Component accessors ---------------------------------------------------

    /// Access the memory subsystem.
    pub fn memory(&self) -> &Arc<dyn Memory> { &self.memory }

    /// Access the agent registry.
    pub fn registry(&self) -> &AgentRegistry { &self.registry }

    /// Access the tool registry.
    pub fn tools(&self) -> &Arc<ToolRegistry> { &self.tools }

    /// Access the prompt repository.
    pub fn prompts(&self) -> &Arc<dyn PromptRepo> { &self.prompts }

    /// Access the event bus.
    pub fn bus(&self) -> &Arc<dyn EventBus> { &self.bus }
}
