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

//! Built-in manifest for the knowledge extraction background pipeline.
//!
//! The extractor is not a user-facing conversational agent — it is a
//! background worker whose LLM binding MUST be resolved through
//! [`crate::llm::DriverRegistry::resolve_agent`]. That path reads
//! `agents.knowledge_extractor.{driver, model}` from YAML so the driver
//! and the model always come from the same source, closing the
//! split-config bug that caused every extraction to 400 in prod
//! (see #1629 / #1636).

use std::sync::LazyLock;

use crate::agent::{AgentManifest, AgentRole, Priority};

/// Canonical agent name for the knowledge extraction pipeline.
///
/// Exposed as a constant so the kernel call site, the boot crate's
/// config validation, and any future consumers agree on the exact key
/// used for `agents.<name>.{driver, model}` YAML lookups.
pub const KNOWLEDGE_EXTRACTOR_NAME: &str = "knowledge_extractor";

static KNOWLEDGE_EXTRACTOR_MANIFEST: LazyLock<AgentManifest> = LazyLock::new(|| AgentManifest {
    name:                   KNOWLEDGE_EXTRACTOR_NAME.to_string(),
    role:                   AgentRole::Worker,
    description:            "Knowledge extraction pipeline — turns conversation tapes into \
                             long-term memory items"
        .to_string(),
    // `model` is deliberately `None`: the concrete model MUST be supplied
    // via `agents.knowledge_extractor.model` YAML so driver + model are
    // resolved atomically through `DriverRegistry::resolve_agent`.
    model:                  None,
    system_prompt:          String::new(),
    soul_prompt:            None,
    provider_hint:          None,
    max_iterations:         Some(1),
    tools:                  vec![],
    excluded_tools:         vec![],
    max_children:           Some(0),
    max_context_tokens:     None,
    priority:               Priority::default(),
    metadata:               serde_json::Value::Null,
    sandbox:                None,
    default_execution_mode: None,
    tool_call_limit:        None,
    worker_timeout_secs:    None,
    max_continuations:      Some(0),
});

/// Return the static knowledge extractor manifest.
pub fn knowledge_extractor_manifest() -> &'static AgentManifest { &KNOWLEDGE_EXTRACTOR_MANIFEST }
