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

//! Data types for the recall strategy engine.

use serde::{Deserialize, Serialize};

/// A recall strategy rule -- registered by agents at runtime, persisted to DB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallRule {
    pub id: String,
    pub name: String,
    pub trigger: Trigger,
    pub action: RecallAction,
    pub inject: InjectTarget,
    pub priority: u16,
    pub enabled: bool,
}

/// Trigger condition -- composable tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Trigger {
    KeywordMatch { keywords: Vec<String> },
    Event { kind: EventKind },
    EveryNTurns { n: u32 },
    InactivityGt { seconds: u64 },
    And { conditions: Vec<Trigger> },
    Or { conditions: Vec<Trigger> },
    Not { condition: Box<Trigger> },
    Always,
}

/// Event kinds that can trigger recall rules.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventKind {
    Compaction,
    NewSession,
    SessionResume,
}

/// Action to perform when a rule's trigger is satisfied.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RecallAction {
    Search { query_template: String, limit: usize },
    DeepRecall { query_template: String },
    GetProfile,
}

/// Where to inject the recall result in the prompt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum InjectTarget {
    SystemPrompt,
    ContextMessage,
}

/// Runtime context passed to the engine for rule evaluation.
#[derive(Debug, Clone)]
pub struct RecallContext {
    pub user_text: String,
    pub turn_count: usize,
    pub events: Vec<EventKind>,
    pub elapsed_since_last_secs: u64,
    pub summary: Option<String>,
    pub session_topic: Option<String>,
}

/// A rule that matched during evaluation, ready for execution.
#[derive(Debug, Clone)]
pub struct MatchedAction {
    pub rule_name: String,
    pub action: RecallAction,
    pub inject: InjectTarget,
    pub priority: u16,
}

/// Result of engine execution -- ready to inject into the prompt.
#[derive(Debug, Clone)]
pub struct InjectionPayload {
    pub rule_name: String,
    pub target: InjectTarget,
    pub content: String,
}

/// Partial update for a recall rule (used by agent tools).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecallRuleUpdate {
    pub trigger: Option<Trigger>,
    pub action: Option<RecallAction>,
    pub inject: Option<InjectTarget>,
    pub priority: Option<u16>,
    pub enabled: Option<bool>,
}
