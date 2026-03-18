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

use rara_kernel::{
    session::SessionKey,
    tool::{ToolContext, ToolOutput},
};
use rara_tool_macro::ToolDef;
use serde_json::json;

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
// Helpers
// ---------------------------------------------------------------------------

/// Generate a unique annotation ID.
fn next_annotation_id() -> String { format!("ann-{}", ulid::Ulid::new()) }

/// Wrap a [`DockMutation`] into a successful [`ToolOutput`].
///
/// Returns a compact confirmation (op + id) rather than the full mutation
/// so that the truncated `result_preview` does not matter.
fn mutation_output(mutation: &DockMutation) -> anyhow::Result<ToolOutput> {
    let summary = json!({
        "ok": true,
        "op": mutation.op,
        "id": mutation.block.as_ref().map(|b| &b.id)
            .or(mutation.fact.as_ref().map(|f| &f.id))
            .or(mutation.annotation.as_ref().map(|a| &a.id))
            .or(mutation.id.as_ref()),
    });
    Ok(summary.into())
}

/// Extract a required string parameter or return an error.
fn required_str(params: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("missing required parameter: {key}"))
}

/// Extract an optional string parameter.
fn optional_str(params: &serde_json::Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// Extract an optional f64 parameter.
fn optional_f64(params: &serde_json::Value, key: &str) -> Option<f64> {
    params.get(key).and_then(|v| v.as_f64())
}

// ===========================================================================
// Block tools
// ===========================================================================

/// Add a single canvas block.
#[derive(ToolDef)]
#[tool(
    name = "dock.block.add",
    description = "Add a new content block to the dock canvas. Returns the mutation to apply.",
    params_schema = "Self::schema_block_add()",
    execute_fn = "self.exec_block_add"
)]
pub struct DockBlockAddTool {
    sink: DockMutationSink,
}

impl DockBlockAddTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }

    fn schema_block_add() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "block_type": {
                    "type": "string",
                    "description": "The type of block (e.g. 'text', 'code', 'image')"
                },
                "html": {
                    "type": "string",
                    "description": "HTML content of the block"
                },
                "id": {
                    "type": "string",
                    "description": "Optional block ID (auto-generated if omitted)"
                }
            },
            "required": ["block_type", "html"]
        })
    }

    async fn exec_block_add(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let block_type = required_str(&params, "block_type")?;
        let html = required_str(&params, "html")?;
        let id = optional_str(&params, "id").unwrap_or_else(next_block_id);

        let mutation = DockMutation {
            op:         MutationOp::BlockAdd,
            actor:      Actor::Agent,
            block:      Some(DockBlock {
                id,
                block_type,
                html,
                diff: None,
            }),
            fact:       None,
            annotation: None,
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        mutation_output(&mutation)
    }
}

/// Update an existing canvas block.
#[derive(ToolDef)]
#[tool(
    name = "dock.block.update",
    description = "Update the HTML content of an existing canvas block.",
    params_schema = "Self::schema_block_update()",
    execute_fn = "self.exec_block_update"
)]
pub struct DockBlockUpdateTool {
    sink: DockMutationSink,
}

impl DockBlockUpdateTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }

    fn schema_block_update() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The block ID to update"
                },
                "html": {
                    "type": "string",
                    "description": "New HTML content for the block"
                }
            },
            "required": ["id", "html"]
        })
    }

    async fn exec_block_update(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let id = required_str(&params, "id")?;
        let html = required_str(&params, "html")?;

        let mutation = DockMutation {
            op:         MutationOp::BlockUpdate,
            actor:      Actor::Agent,
            block:      Some(DockBlock {
                id,
                block_type: String::new(),
                html,
                diff: None,
            }),
            fact:       None,
            annotation: None,
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        mutation_output(&mutation)
    }
}

/// Remove a canvas block.
#[derive(ToolDef)]
#[tool(
    name = "dock.block.remove",
    description = "Remove a canvas block by ID.",
    params_schema = "Self::schema_block_remove()",
    execute_fn = "self.exec_block_remove"
)]
pub struct DockBlockRemoveTool {
    sink: DockMutationSink,
}

impl DockBlockRemoveTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }

    fn schema_block_remove() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The block ID to remove"
                }
            },
            "required": ["id"]
        })
    }

    async fn exec_block_remove(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let id = required_str(&params, "id")?;

        let mutation = DockMutation {
            op:         MutationOp::BlockRemove,
            actor:      Actor::Agent,
            block:      None,
            fact:       None,
            annotation: None,
            id:         Some(id),
        };
        self.sink.push(context.session_key, mutation.clone());
        mutation_output(&mutation)
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
    params_schema = "Self::schema_fact_add()",
    execute_fn = "self.exec_fact_add"
)]
pub struct DockFactAddTool {
    sink: DockMutationSink,
}

impl DockFactAddTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }

    fn schema_fact_add() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The fact content"
                },
                "source": {
                    "type": "string",
                    "description": "Optional source label (defaults to 'agent')"
                }
            },
            "required": ["content"]
        })
    }

    async fn exec_fact_add(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let content = required_str(&params, "content")?;
        let id = next_fact_id();

        let mutation = DockMutation {
            op:         MutationOp::FactAdd,
            actor:      Actor::Agent,
            block:      None,
            fact:       Some(DockFact {
                id,
                content,
                source: Actor::Agent,
            }),
            annotation: None,
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        mutation_output(&mutation)
    }
}

/// Update an existing shared fact.
#[derive(ToolDef)]
#[tool(
    name = "dock.fact.update",
    description = "Update the content of an existing shared fact.",
    params_schema = "Self::schema_fact_update()",
    execute_fn = "self.exec_fact_update"
)]
pub struct DockFactUpdateTool {
    sink: DockMutationSink,
}

impl DockFactUpdateTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }

    fn schema_fact_update() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The fact ID to update"
                },
                "content": {
                    "type": "string",
                    "description": "New fact content"
                }
            },
            "required": ["id", "content"]
        })
    }

    async fn exec_fact_update(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let id = required_str(&params, "id")?;
        let content = required_str(&params, "content")?;

        let mutation = DockMutation {
            op:         MutationOp::FactUpdate,
            actor:      Actor::Agent,
            block:      None,
            fact:       Some(DockFact {
                id,
                content,
                source: Actor::Agent,
            }),
            annotation: None,
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        mutation_output(&mutation)
    }
}

/// Remove a shared fact.
#[derive(ToolDef)]
#[tool(
    name = "dock.fact.remove",
    description = "Remove a shared fact by ID.",
    params_schema = "Self::schema_fact_remove()",
    execute_fn = "self.exec_fact_remove"
)]
pub struct DockFactRemoveTool {
    sink: DockMutationSink,
}

impl DockFactRemoveTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }

    fn schema_fact_remove() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The fact ID to remove"
                }
            },
            "required": ["id"]
        })
    }

    async fn exec_fact_remove(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let id = required_str(&params, "id")?;

        let mutation = DockMutation {
            op:         MutationOp::FactRemove,
            actor:      Actor::Agent,
            block:      None,
            fact:       None,
            annotation: None,
            id:         Some(id),
        };
        self.sink.push(context.session_key, mutation.clone());
        mutation_output(&mutation)
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
    params_schema = "Self::schema_ann_add()",
    execute_fn = "self.exec_ann_add"
)]
pub struct DockAnnotationAddTool {
    sink: DockMutationSink,
}

impl DockAnnotationAddTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }

    fn schema_ann_add() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Annotation text content"
                },
                "block_id": {
                    "type": "string",
                    "description": "Optional block to attach the annotation to"
                },
                "selection_text": {
                    "type": "string",
                    "description": "Optional selected text within the block"
                },
                "anchor_y": {
                    "type": "number",
                    "description": "Optional vertical anchor position"
                },
                "id": {
                    "type": "string",
                    "description": "Optional annotation ID (auto-generated if omitted)"
                }
            },
            "required": ["content"]
        })
    }

    async fn exec_ann_add(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let content = required_str(&params, "content")?;
        let block_id = optional_str(&params, "block_id").unwrap_or_default();
        let anchor_y = optional_f64(&params, "anchor_y").unwrap_or(0.0);
        let id = optional_str(&params, "id").unwrap_or_else(next_annotation_id);

        let selection =
            optional_str(&params, "selection_text").map(|text| crate::models::DockSelection {
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
                content,
                author: Actor::Agent,
                anchor_y,
                timestamp: now_ms,
                selection,
            }),
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        mutation_output(&mutation)
    }
}

/// Update an existing annotation.
#[derive(ToolDef)]
#[tool(
    name = "dock.annotation.update",
    description = "Update an existing annotation.",
    params_schema = "Self::schema_ann_update()",
    execute_fn = "self.exec_ann_update"
)]
pub struct DockAnnotationUpdateTool {
    sink: DockMutationSink,
}

impl DockAnnotationUpdateTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }

    fn schema_ann_update() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The annotation ID to update"
                },
                "content": {
                    "type": "string",
                    "description": "New annotation content"
                },
                "block_id": {
                    "type": "string",
                    "description": "Optional new block attachment"
                },
                "selection_text": {
                    "type": "string",
                    "description": "Optional updated selection text"
                },
                "anchor_y": {
                    "type": "number",
                    "description": "Optional updated vertical anchor"
                }
            },
            "required": ["id", "content"]
        })
    }

    async fn exec_ann_update(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let id = required_str(&params, "id")?;
        let content = required_str(&params, "content")?;
        let block_id = optional_str(&params, "block_id").unwrap_or_default();
        let anchor_y = optional_f64(&params, "anchor_y").unwrap_or(0.0);

        let selection =
            optional_str(&params, "selection_text").map(|text| crate::models::DockSelection {
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
                id,
                block_id,
                content,
                author: Actor::Agent,
                anchor_y,
                timestamp: now_ms,
                selection,
            }),
            id:         None,
        };
        self.sink.push(context.session_key, mutation.clone());
        mutation_output(&mutation)
    }
}

/// Remove an annotation.
#[derive(ToolDef)]
#[tool(
    name = "dock.annotation.remove",
    description = "Remove an annotation by ID.",
    params_schema = "Self::schema_ann_remove()",
    execute_fn = "self.exec_ann_remove"
)]
pub struct DockAnnotationRemoveTool {
    sink: DockMutationSink,
}

impl DockAnnotationRemoveTool {
    /// Create with a shared mutation sink.
    pub fn new(sink: DockMutationSink) -> Self { Self { sink } }

    fn schema_ann_remove() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The annotation ID to remove"
                }
            },
            "required": ["id"]
        })
    }

    async fn exec_ann_remove(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let id = required_str(&params, "id")?;

        let mutation = DockMutation {
            op:         MutationOp::AnnotationRemove,
            actor:      Actor::Agent,
            block:      None,
            fact:       None,
            annotation: None,
            id:         Some(id),
        };
        self.sink.push(context.session_key, mutation.clone());
        mutation_output(&mutation)
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
