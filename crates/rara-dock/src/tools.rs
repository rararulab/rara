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

//! Dock agent tools — CRUD operations for blocks, facts, and annotations.
//!
//! Each tool constructs a [`DockMutation`], pushes it into a shared
//! [`DockMutationSink`] keyed by kernel `SessionKey`, and returns a
//! confirmation to the LLM.  The turn handler drains the sink after
//! the agent loop completes and applies mutations to the session store.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use rara_kernel::{
    session::SessionKey,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    models::{Actor, DockAnnotation, DockBlock, DockFact, DockMutation, MutationOp},
    state::{next_block_id, next_fact_id},
};

// ---------------------------------------------------------------------------
// Mutation sink — tools push here, turn handler drains
// ---------------------------------------------------------------------------

/// Thread-safe sink for dock mutations produced by agent tools.
///
/// Keyed by kernel `SessionKey` so the turn handler can drain mutations
/// for a specific session after the turn completes, bypassing the
/// truncated `result_preview` stream.
#[derive(Clone, Default)]
pub struct DockMutationSink {
    inner: Arc<Mutex<HashMap<SessionKey, Vec<DockMutation>>>>,
}

impl DockMutationSink {
    /// Create an empty sink.
    pub fn new() -> Self { Self::default() }

    /// Push a mutation for a given session.
    pub fn push(&self, key: SessionKey, mutation: DockMutation) {
        self.inner
            .lock()
            .expect("DockMutationSink lock poisoned")
            .entry(key)
            .or_default()
            .push(mutation);
    }

    /// Drain all mutations for a session, returning them in insertion order.
    pub fn drain(&self, key: &SessionKey) -> Vec<DockMutation> {
        self.inner
            .lock()
            .expect("DockMutationSink lock poisoned")
            .remove(key)
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Shared result type
// ---------------------------------------------------------------------------

/// Generate a unique annotation ID.
fn next_annotation_id() -> String { format!("ann-{}", ulid::Ulid::new()) }

/// Compact confirmation returned by all dock mutation tools.
#[derive(Debug, Clone, Serialize)]
pub struct DockMutationResult {
    /// Whether the mutation was accepted.
    ok: bool,
    /// The mutation operation performed.
    op: MutationOp,
    /// The ID of the affected entity.
    id: String,
}

impl DockMutationResult {
    /// Build a success result from a completed mutation.
    fn from_mutation(mutation: &DockMutation) -> Self {
        let id = mutation
            .block
            .as_ref()
            .map(|b| b.id.clone())
            .or_else(|| mutation.fact.as_ref().map(|f| f.id.clone()))
            .or_else(|| mutation.annotation.as_ref().map(|a| a.id.clone()))
            .or_else(|| mutation.id.clone())
            .unwrap_or_default();
        Self {
            ok: true,
            op: mutation.op.clone(),
            id,
        }
    }
}

// ===========================================================================
// Block tools
// ===========================================================================

/// Add a single canvas block.
#[derive(ToolDef)]
#[tool(
    name = "dock.block.add",
    description = "Add a new content block to the dock canvas. Returns the mutation to apply.",
    tier = "deferred"
)]
pub struct DockBlockAddTool {
    sink: DockMutationSink,
}

impl DockBlockAddTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }
}

/// Parameters for `dock.block.add`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockAddParams {
    /// The type of block (e.g. "text", "code", "image").
    block_type: String,
    /// HTML content of the block.
    html:       String,
    /// Optional block ID (auto-generated if omitted).
    id:         Option<String>,
}

#[async_trait]
impl ToolExecute for DockBlockAddTool {
    type Output = DockMutationResult;
    type Params = BlockAddParams;

    async fn run(
        &self,
        p: BlockAddParams,
        context: &ToolContext,
    ) -> anyhow::Result<DockMutationResult> {
        let id = p.id.unwrap_or_else(next_block_id);
        let mutation = DockMutation {
            op:         MutationOp::BlockAdd,
            actor:      Actor::Agent,
            block:      Some(DockBlock {
                id,
                block_type: p.block_type,
                html: p.html,
                diff: None,
            }),
            fact:       None,
            annotation: None,
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        Ok(DockMutationResult::from_mutation(&mutation))
    }
}

/// Update an existing canvas block.
#[derive(ToolDef)]
#[tool(
    name = "dock.block.update",
    description = "Update the HTML content of an existing canvas block.",
    tier = "deferred"
)]
pub struct DockBlockUpdateTool {
    sink: DockMutationSink,
}

impl DockBlockUpdateTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }
}

/// Parameters for `dock.block.update`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockUpdateParams {
    /// The block ID to update.
    id:   String,
    /// New HTML content for the block.
    html: String,
}

#[async_trait]
impl ToolExecute for DockBlockUpdateTool {
    type Output = DockMutationResult;
    type Params = BlockUpdateParams;

    async fn run(
        &self,
        p: BlockUpdateParams,
        context: &ToolContext,
    ) -> anyhow::Result<DockMutationResult> {
        let mutation = DockMutation {
            op:         MutationOp::BlockUpdate,
            actor:      Actor::Agent,
            block:      Some(DockBlock {
                id:         p.id,
                block_type: String::new(),
                html:       p.html,
                diff:       None,
            }),
            fact:       None,
            annotation: None,
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        Ok(DockMutationResult::from_mutation(&mutation))
    }
}

/// Remove a canvas block.
#[derive(ToolDef)]
#[tool(
    name = "dock.block.remove",
    description = "Remove a canvas block by ID.",
    tier = "deferred"
)]
pub struct DockBlockRemoveTool {
    sink: DockMutationSink,
}

impl DockBlockRemoveTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }
}

/// Parameters for `dock.block.remove`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockRemoveParams {
    /// The block ID to remove.
    id: String,
}

#[async_trait]
impl ToolExecute for DockBlockRemoveTool {
    type Output = DockMutationResult;
    type Params = BlockRemoveParams;

    async fn run(
        &self,
        p: BlockRemoveParams,
        context: &ToolContext,
    ) -> anyhow::Result<DockMutationResult> {
        let mutation = DockMutation {
            op:         MutationOp::BlockRemove,
            actor:      Actor::Agent,
            block:      None,
            fact:       None,
            annotation: None,
            id:         Some(p.id),
        };
        self.sink.push(context.session_key, mutation.clone());
        Ok(DockMutationResult::from_mutation(&mutation))
    }
}

// ===========================================================================
// Fact tools
// ===========================================================================

/// Add a shared fact to the dock session.
#[derive(ToolDef)]
#[tool(
    name = "dock.fact.add",
    description = "Add a shared fact to the dock session. Facts persist across turns.",
    tier = "deferred"
)]
pub struct DockFactAddTool {
    sink: DockMutationSink,
}

impl DockFactAddTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }
}

/// Parameters for `dock.fact.add`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FactAddParams {
    /// The fact content.
    content: String,
    /// Optional source label (defaults to "agent").
    source:  Option<String>,
}

#[async_trait]
impl ToolExecute for DockFactAddTool {
    type Output = DockMutationResult;
    type Params = FactAddParams;

    async fn run(
        &self,
        p: FactAddParams,
        context: &ToolContext,
    ) -> anyhow::Result<DockMutationResult> {
        let id = next_fact_id();
        let mutation = DockMutation {
            op:         MutationOp::FactAdd,
            actor:      Actor::Agent,
            block:      None,
            fact:       Some(DockFact {
                id,
                content: p.content,
                source: Actor::Agent,
            }),
            annotation: None,
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        Ok(DockMutationResult::from_mutation(&mutation))
    }
}

/// Update an existing shared fact.
#[derive(ToolDef)]
#[tool(
    name = "dock.fact.update",
    description = "Update the content of an existing shared fact.",
    tier = "deferred"
)]
pub struct DockFactUpdateTool {
    sink: DockMutationSink,
}

impl DockFactUpdateTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }
}

/// Parameters for `dock.fact.update`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FactUpdateParams {
    /// The fact ID to update.
    id:      String,
    /// New fact content.
    content: String,
}

#[async_trait]
impl ToolExecute for DockFactUpdateTool {
    type Output = DockMutationResult;
    type Params = FactUpdateParams;

    async fn run(
        &self,
        p: FactUpdateParams,
        context: &ToolContext,
    ) -> anyhow::Result<DockMutationResult> {
        let mutation = DockMutation {
            op:         MutationOp::FactUpdate,
            actor:      Actor::Agent,
            block:      None,
            fact:       Some(DockFact {
                id:      p.id,
                content: p.content,
                source:  Actor::Agent,
            }),
            annotation: None,
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        Ok(DockMutationResult::from_mutation(&mutation))
    }
}

/// Remove a shared fact.
#[derive(ToolDef)]
#[tool(
    name = "dock.fact.remove",
    description = "Remove a shared fact by ID.",
    tier = "deferred"
)]
pub struct DockFactRemoveTool {
    sink: DockMutationSink,
}

impl DockFactRemoveTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }
}

/// Parameters for `dock.fact.remove`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FactRemoveParams {
    /// The fact ID to remove.
    id: String,
}

#[async_trait]
impl ToolExecute for DockFactRemoveTool {
    type Output = DockMutationResult;
    type Params = FactRemoveParams;

    async fn run(
        &self,
        p: FactRemoveParams,
        context: &ToolContext,
    ) -> anyhow::Result<DockMutationResult> {
        let mutation = DockMutation {
            op:         MutationOp::FactRemove,
            actor:      Actor::Agent,
            block:      None,
            fact:       None,
            annotation: None,
            id:         Some(p.id),
        };
        self.sink.push(context.session_key, mutation.clone());
        Ok(DockMutationResult::from_mutation(&mutation))
    }
}

// ===========================================================================
// Annotation tools
// ===========================================================================

/// Add an annotation to the dock session.
#[derive(ToolDef)]
#[tool(
    name = "dock.annotation.add",
    description = "Add an annotation to the dock canvas, optionally attached to a block.",
    tier = "deferred"
)]
pub struct DockAnnotationAddTool {
    sink: DockMutationSink,
}

impl DockAnnotationAddTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }
}

/// Parameters for `dock.annotation.add`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnnotationAddParams {
    /// Annotation text content.
    content:        String,
    /// Optional block to attach the annotation to.
    block_id:       Option<String>,
    /// Optional selected text within the block.
    selection_text: Option<String>,
    /// Optional vertical anchor position.
    anchor_y:       Option<f64>,
    /// Optional annotation ID (auto-generated if omitted).
    id:             Option<String>,
}

#[async_trait]
impl ToolExecute for DockAnnotationAddTool {
    type Output = DockMutationResult;
    type Params = AnnotationAddParams;

    async fn run(
        &self,
        p: AnnotationAddParams,
        context: &ToolContext,
    ) -> anyhow::Result<DockMutationResult> {
        let block_id = p.block_id.unwrap_or_default();
        let anchor_y = p.anchor_y.unwrap_or(0.0);
        let id = p.id.unwrap_or_else(next_annotation_id);

        let selection = p.selection_text.map(|text| crate::models::DockSelection {
            start: 0,
            end: text.len(),
            text,
        });

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let mutation = DockMutation {
            op:         MutationOp::AnnotationAdd,
            actor:      Actor::Agent,
            block:      None,
            fact:       None,
            annotation: Some(DockAnnotation {
                id,
                block_id,
                content: p.content,
                author: Actor::Agent,
                anchor_y,
                timestamp: now_ms,
                selection,
            }),
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        Ok(DockMutationResult::from_mutation(&mutation))
    }
}

/// Update an existing annotation.
#[derive(ToolDef)]
#[tool(
    name = "dock.annotation.update",
    description = "Update an existing annotation.",
    tier = "deferred"
)]
pub struct DockAnnotationUpdateTool {
    sink: DockMutationSink,
}

impl DockAnnotationUpdateTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }
}

/// Parameters for `dock.annotation.update`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnnotationUpdateParams {
    /// The annotation ID to update.
    id:             String,
    /// New annotation content.
    content:        String,
    /// Optional new block attachment.
    block_id:       Option<String>,
    /// Optional updated selection text.
    selection_text: Option<String>,
    /// Optional updated vertical anchor.
    anchor_y:       Option<f64>,
}

#[async_trait]
impl ToolExecute for DockAnnotationUpdateTool {
    type Output = DockMutationResult;
    type Params = AnnotationUpdateParams;

    async fn run(
        &self,
        p: AnnotationUpdateParams,
        context: &ToolContext,
    ) -> anyhow::Result<DockMutationResult> {
        let block_id = p.block_id.unwrap_or_default();
        let anchor_y = p.anchor_y.unwrap_or(0.0);

        let selection = p.selection_text.map(|text| crate::models::DockSelection {
            start: 0,
            end: text.len(),
            text,
        });

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let mutation = DockMutation {
            op:         MutationOp::AnnotationUpdate,
            actor:      Actor::Agent,
            block:      None,
            fact:       None,
            annotation: Some(DockAnnotation {
                id: p.id,
                block_id,
                content: p.content,
                author: Actor::Agent,
                anchor_y,
                timestamp: now_ms,
                selection,
            }),
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        Ok(DockMutationResult::from_mutation(&mutation))
    }
}

/// Remove an annotation.
#[derive(ToolDef)]
#[tool(
    name = "dock.annotation.remove",
    description = "Remove an annotation by ID.",
    tier = "deferred"
)]
pub struct DockAnnotationRemoveTool {
    sink: DockMutationSink,
}

impl DockAnnotationRemoveTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }
}

/// Parameters for `dock.annotation.remove`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnnotationRemoveParams {
    /// The annotation ID to remove.
    id: String,
}

#[async_trait]
impl ToolExecute for DockAnnotationRemoveTool {
    type Output = DockMutationResult;
    type Params = AnnotationRemoveParams;

    async fn run(
        &self,
        p: AnnotationRemoveParams,
        context: &ToolContext,
    ) -> anyhow::Result<DockMutationResult> {
        let mutation = DockMutation {
            op:         MutationOp::AnnotationRemove,
            actor:      Actor::Agent,
            block:      None,
            fact:       None,
            annotation: None,
            id:         Some(p.id),
        };
        self.sink.push(context.session_key, mutation.clone());
        Ok(DockMutationResult::from_mutation(&mutation))
    }
}

// ---------------------------------------------------------------------------
// Convenience: collect all dock tools
// ---------------------------------------------------------------------------

/// Return all 9 dock tools as a vec of `AgentToolRef`.
///
/// All tools share the given [`DockMutationSink`] so that the turn handler
/// can drain full mutations after the agent loop completes.
pub fn dock_tools(sink: DockMutationSink) -> Vec<rara_kernel::tool::AgentToolRef> {
    vec![
        Arc::new(DockBlockAddTool::new(sink.clone())),
        Arc::new(DockBlockUpdateTool::new(sink.clone())),
        Arc::new(DockBlockRemoveTool::new(sink.clone())),
        Arc::new(DockFactAddTool::new(sink.clone())),
        Arc::new(DockFactUpdateTool::new(sink.clone())),
        Arc::new(DockFactRemoveTool::new(sink.clone())),
        Arc::new(DockAnnotationAddTool::new(sink.clone())),
        Arc::new(DockAnnotationUpdateTool::new(sink.clone())),
        Arc::new(DockAnnotationRemoveTool::new(sink)),
    ]
}

/// All dock tool names for manifest registration.
pub fn dock_tool_names() -> Vec<&'static str> {
    vec![
        DockBlockAddTool::TOOL_NAME,
        DockBlockUpdateTool::TOOL_NAME,
        DockBlockRemoveTool::TOOL_NAME,
        DockFactAddTool::TOOL_NAME,
        DockFactUpdateTool::TOOL_NAME,
        DockFactRemoveTool::TOOL_NAME,
        DockAnnotationAddTool::TOOL_NAME,
        DockAnnotationUpdateTool::TOOL_NAME,
        DockAnnotationRemoveTool::TOOL_NAME,
    ]
}
