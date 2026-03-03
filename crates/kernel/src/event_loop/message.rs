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

//! User message routing — 3-path routing logic, endpoint registration,
//! and message delivery.

use tracing::{error, info, info_span, warn};

use super::runtime::RuntimeTable;
use crate::{
    event::KernelEvent,
    io::types::{InboundMessage, OutboundEnvelope},
    kernel::Kernel,
    process::{AgentId, ProcessState, principal::Principal},
};

impl Kernel {
    /// Handle a user message with 3-path routing:
    ///
    /// 1. **ID addressing** (`target_agent_id` set): deliver to specific
    ///    process — error if terminal or not found (A2A Protocol pattern).
    /// 2. **Session addressing** (session_index match): deliver to bound
    ///    process — if terminal, clear binding and respawn transparently
    ///    (AutoGen lazy instantiation pattern).
    /// 3. **Name addressing** (fallback): lookup AgentRegistry by name, always
    ///    spawn a new process (Anthropic spawn-new pattern).
    pub(crate) async fn handle_user_message(&self, msg: InboundMessage, runtimes: &RuntimeTable) {
        let span = info_span!(
            "handle_user_message",
            session_id = %msg.session_id,
            user_id = %msg.user.0,
            channel = ?msg.source.channel_type,
            routing_path = tracing::field::Empty,
        );
        let _guard = span.enter();

        let session_id = msg.session_id.clone();
        let user = msg.user.clone();

        self.register_stateless_endpoint(&msg);

        // ----- Path 1: ID addressing (agent-to-agent) -----
        if let Some(target_id) = msg.target_agent_id {
            span.record("routing_path", "id_addressing");
            match self.process_table().get(target_id) {
                Some(process) if process.state.is_terminal() => {
                    let envelope = OutboundEnvelope::error(
                        msg.id.clone(),
                        user.clone(),
                        session_id.clone(),
                        "process_terminal",
                        format!("process {} is {}", target_id, process.state),
                    );
                    if let Err(e) = self.event_queue().try_push(KernelEvent::Deliver(envelope)) {
                        error!(%e, "failed to push process-terminal error Deliver");
                    }
                    return;
                }
                Some(_) => {
                    self.deliver_to_process(target_id, msg, runtimes).await;
                    return;
                }
                None => {
                    let envelope = OutboundEnvelope::error(
                        msg.id.clone(),
                        user.clone(),
                        session_id.clone(),
                        "process_not_found",
                        format!("process not found: {target_id}"),
                    );
                    if let Err(e) = self.event_queue().try_push(KernelEvent::Deliver(envelope)) {
                        error!(%e, "failed to push process-not-found error Deliver");
                    }
                    return;
                }
            }
        }

        // ----- Path 2: Session addressing (external user) -----
        if let Some(process) = self.process_table().find_by_session(&session_id) {
            span.record("routing_path", "session_addressing");
            let aid = process.agent_id;

            if process.state.is_terminal() {
                // Terminal process — clear session binding, fall through to
                // Path 3 (Name addressing) to spawn a replacement.
                // We do NOT remove the process from the table here — the
                // reaper (lazy cleanup in all_process_stats) handles that
                // after the TTL expires.
                info!(
                    agent_id = %aid,
                    session_id = %session_id,
                    state = %process.state,
                    "session-bound process terminal — clearing binding, will respawn"
                );
                if let Some(ref channel_sid) = process.channel_session_id {
                    self.process_table().session_index_remove(channel_sid, aid);
                }
                // Fall through to Path 3 below.
            } else {
                self.deliver_to_process(aid, msg, runtimes).await;
                return;
            }
        }

        // ----- Path 3: Name addressing (always spawn new) -----
        span.record("routing_path", "name_addressing");
        let target_name = if let Some(name) = msg.target_agent.as_deref() {
            name.to_string()
        } else {
            self.default_agent_for_user(&msg.user).await
        };

        let manifest = if let Some(m) = self.agent_registry().get(&target_name) {
            m
        } else if target_name == Self::ADMIN_AGENT_NAME {
            match self.resolve_manifest_for_auto_spawn().await {
                Some(m) => m,
                None => {
                    error!(
                        session_id = %session_id,
                        "no model configured — cannot spawn root agent"
                    );
                    return;
                }
            }
        } else {
            warn!(
                target_name = %target_name,
                session_id = %session_id,
                "unknown target agent"
            );
            let envelope = OutboundEnvelope::error(
                msg.id.clone(),
                user.clone(),
                session_id.clone(),
                "unknown_agent",
                format!("unknown target agent: {target_name}"),
            );
            if let Err(e) = self.event_queue().try_push(KernelEvent::Deliver(envelope)) {
                error!(%e, "failed to push unknown-agent error Deliver");
            }
            return;
        };

        let principal = Principal::user(user.0.clone());
        match self
            .handle_spawn_agent(
                manifest,
                msg.content.as_text(),
                principal,
                Some(session_id.clone()),
                None,
                runtimes,
            )
            .await
        {
            Ok(_aid) => {
                // handle_spawn_agent pushes a synthetic UserMessage that will
                // re-enter handle_user_message and be routed via Path 2.
            }
            Err(e) => {
                error!(session_id = %session_id, error = %e, "failed to spawn agent");
            }
        }
    }

    /// Register egress endpoint for stateless channels (e.g. Telegram).
    ///
    /// Connection-oriented channels (Web) register on WS/SSE connect.
    /// Stateless channels have no persistent connection, so we register on
    /// every inbound message (idempotent — EndpointRegistry uses a HashSet).
    fn register_stateless_endpoint(&self, msg: &InboundMessage) {
        if msg.source.channel_type != crate::channel::types::ChannelType::Telegram {
            return;
        }
        let Some(ref chat_id_str) = msg.source.platform_chat_id else {
            return;
        };
        let Ok(chat_id) = chat_id_str.parse::<i64>() else {
            return;
        };
        self.endpoint_registry().register(
            &msg.user,
            crate::io::egress::Endpoint {
                channel_type: crate::channel::types::ChannelType::Telegram,
                address:      crate::io::egress::EndpointAddress::Telegram {
                    chat_id,
                    thread_id: None,
                },
            },
        );
    }

    /// Deliver a message to a live process: buffer if the process is paused
    /// or busy (Running state), otherwise start a new LLM turn.
    pub(crate) async fn deliver_to_process(
        &self,
        agent_id: AgentId,
        msg: InboundMessage,
        runtimes: &RuntimeTable,
    ) {
        if let Some(mut rt) = runtimes.get_mut(&agent_id) {
            if rt.paused {
                rt.pause_buffer.push(KernelEvent::UserMessage(msg));
                return;
            }
            if let Some(p) = self.process_table().get(agent_id) {
                if p.state == ProcessState::Running {
                    rt.pause_buffer.push(KernelEvent::UserMessage(msg));
                    return;
                }
            }
        }
        self.start_llm_turn(agent_id, msg, runtimes).await;
    }

    /// Determine the default agent name for a user based on their role.
    ///
    /// - Root / Admin users -> "rara" (full-capability agent)
    /// - Regular users -> "nana" (chat-only companion)
    /// - Unknown users -> "nana" (safe default)
    pub(crate) async fn default_agent_for_user(
        &self,
        user: &crate::process::principal::UserId,
    ) -> String {
        use crate::process::principal::Role;

        match self.security().resolve_user_role(user).await {
            Role::Root | Role::Admin => Self::ADMIN_AGENT_NAME.to_string(),
            Role::User => Self::USER_AGENT_NAME.to_string(),
        }
    }

    /// Resolve a manifest for auto-spawning (when a user message arrives
    /// with no existing process).
    pub(crate) async fn resolve_manifest_for_auto_spawn(
        &self,
    ) -> Option<crate::process::AgentManifest> {
        let model = rara_domain_shared::settings::get_model(self.settings().as_ref(), "chat").await;
        Some(crate::process::AgentManifest {
            name: "io-agent".to_string(),
            role: None,
            description: "I/O bus agent".to_string(),
            model,
            system_prompt: "You are a helpful assistant.".to_string(),
            soul_prompt: None,
            provider_hint: None,
            max_iterations: Some(25),
            tools: vec![],
            max_children: None,
            max_context_tokens: None,
            priority: crate::process::Priority::default(),
            metadata: serde_json::Value::Null,
            sandbox: None,
        })
    }
}
