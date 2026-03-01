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

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Identity ────────────────────────────────────────────────────────

/// Identity context for the entity performing memory operations.
#[derive(Debug, Clone)]
pub struct MemoryContext {
    /// Who is the end-user.
    pub user_id:    Uuid,
    /// Which agent is operating.
    pub agent_id:   Uuid,
    /// Current session / run (if any).
    pub session_id: Option<Uuid>,
}

// ─── Scope ───────────────────────────────────────────────────────────

/// Visibility partition for memory records.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Shared across all agents.
    Global,
    /// Shared within a named team / project.
    Team(String),
    /// Private to the agent identified by `MemoryContext::agent_id`.
    Agent,
}

/// Visibility partition for KV shared memory operations.
///
/// Used by `ProcessHandle::shared_store` and `ProcessHandle::shared_recall`
/// to provide cross-agent data sharing with explicit scope control.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KvScope {
    /// Global scope — key stored as-is. Requires Root or Admin role.
    Global,
    /// Team scope — key prefixed with `"team:{name}:"`. Requires Root or
    /// Admin role.
    Team(String),
    /// Agent scope — key prefixed with `"agent:{agent_id}:"`. Regular agents
    /// can only access their own agent scope; Root/Admin can access any.
    Agent(uuid::Uuid),
}

// ─── State Layer types ───────────────────────────────────────────────

/// A conversation message fed into
/// [`StateMemory::add`](super::StateMemory::add).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role:    String,
    pub content: String,
}

/// A single structured fact extracted and maintained by the state layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFact {
    pub id:         Uuid,
    pub content:    String,
    #[serde(default)]
    pub score:      Option<f64>,
    #[serde(default)]
    pub metadata:   Option<serde_json::Value>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
}

/// Result of a single [`StateMemory::add`](super::StateMemory::add) event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEvent {
    pub id:               Uuid,
    /// `ADD`, `UPDATE`, `DELETE`, or `NOOP`.
    pub event:            String,
    pub content:          String,
    #[serde(default)]
    pub previous_content: Option<String>,
}

/// One entry in the change history of a [`StateFact`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateHistory {
    pub id:          Uuid,
    pub memory_id:   Uuid,
    #[serde(default)]
    pub old_content: Option<String>,
    #[serde(default)]
    pub new_content: Option<String>,
    pub event:       String,
    #[serde(default)]
    pub created_at:  Option<Timestamp>,
    pub is_deleted:  bool,
}

// ─── Knowledge Layer types ───────────────────────────────────────────

/// A persistent knowledge note managed by the knowledge layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeNote {
    pub id:         Uuid,
    pub content:    String,
    pub tags:       Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

// ─── Learning Layer types ────────────────────────────────────────────

/// A single entry recalled from the learning layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallEntry {
    pub id:      Uuid,
    pub content: String,
    pub score:   f64,
}
