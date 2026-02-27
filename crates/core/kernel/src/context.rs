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

//! Agent context — the component bundle that Kernel creates for each agent
//! invocation.

use std::sync::Arc;

use uuid::Uuid;

use crate::{
    event::EventBus,
    guard::Guard,
    llm::LlmApi,
    memory::Memory,
    prompt::PromptRepo,
    session::SessionStore,
    tool::ToolRegistry,
};

/// Runtime context for an agent invocation.
///
/// Contains identity information, references to all 7 kernel components, and
/// per-agent configuration. Created by [`Kernel::create_context`] and passed
/// to the execution loop.
pub struct RunContext {
    // -- Identity --
    pub agent_id:   Uuid,
    pub session_id: Uuid,
    pub user_id:    Uuid,

    // -- 7 Component references --
    pub llm:      Arc<dyn LlmApi>,
    pub tools:    Arc<ToolRegistry>,
    pub memory:   Arc<dyn Memory>,
    pub sessions: Arc<dyn SessionStore>,
    pub prompts:  Arc<dyn PromptRepo>,
    pub guard:    Arc<dyn Guard>,
    pub bus:      Arc<dyn EventBus>,

    // -- Agent configuration --
    pub model:          String,
    pub system_prompt:  String,
    pub max_iterations: usize,
}
