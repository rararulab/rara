use serde::{Deserialize, Serialize};

/// The actor who performed an action: human user or AI agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    Human,
    Agent,
}

// ---------------------------------------------------------------------------
// Core entities
// ---------------------------------------------------------------------------

/// A single content block on the canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockBlock {
    pub id:         String,
    pub block_type: String,
    pub html:       String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff:       Option<DockDiff>,
}

/// A diff attached to a block showing before/after.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockDiff {
    pub original: String,
    pub modified: String,
    pub author:   Actor,
}

/// A persistent fact displayed in the facts panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockFact {
    pub id:      String,
    pub content: String,
    pub source:  Actor,
}

/// A text selection range within a block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockSelection {
    pub start: usize,
    pub end:   usize,
    pub text:  String,
}

/// An annotation attached to a canvas block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockAnnotation {
    pub id:        String,
    pub block_id:  String,
    pub content:   String,
    pub author:    Actor,
    pub anchor_y:  f64,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<DockSelection>,
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

/// The kind of mutation to apply to the session document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationOp {
    #[serde(rename = "session.upsert")]
    SessionUpsert,
    #[serde(rename = "block.add")]
    BlockAdd,
    #[serde(rename = "block.update")]
    BlockUpdate,
    #[serde(rename = "block.remove")]
    BlockRemove,
    #[serde(rename = "fact.add")]
    FactAdd,
    #[serde(rename = "fact.update")]
    FactUpdate,
    #[serde(rename = "fact.remove")]
    FactRemove,
    #[serde(rename = "annotation.add")]
    AnnotationAdd,
    #[serde(rename = "annotation.update")]
    AnnotationUpdate,
    #[serde(rename = "annotation.remove")]
    AnnotationRemove,
}

/// A single mutation instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockMutation {
    pub op:         MutationOp,
    pub actor:      Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block:      Option<DockBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fact:       Option<DockFact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<DockAnnotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id:         Option<String>,
}

/// A batch of mutations to apply atomically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockMutationBatch {
    pub mutations: Vec<DockMutation>,
}

// ---------------------------------------------------------------------------
// Session / Document
// ---------------------------------------------------------------------------

/// Metadata for a dock session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockSessionMeta {
    pub id:              String,
    #[serde(default)]
    pub title:           String,
    #[serde(default)]
    pub preview:         String,
    #[serde(default)]
    pub created_at:      i64,
    #[serde(default)]
    pub updated_at:      i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_anchor: Option<String>,
}

/// The full persisted document for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockSessionDocument {
    pub session:     DockSessionMeta,
    #[serde(default)]
    pub blocks:      Vec<DockBlock>,
    #[serde(default)]
    pub annotations: Vec<DockAnnotation>,
    #[serde(default)]
    pub facts:       Vec<DockFact>,
}

/// An in-memory snapshot of the canvas state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockCanvasSnapshot {
    #[serde(default)]
    pub blocks: Vec<DockBlock>,
    #[serde(default)]
    pub facts:  Vec<DockFact>,
}

/// Workspace-level state (which session is active).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockWorkspaceState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_session_id: Option<String>,
}

// ---------------------------------------------------------------------------
// API request / response types
// ---------------------------------------------------------------------------

/// Response returned when bootstrapping the dock UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockBootstrapResponse {
    pub sessions:          Vec<DockSessionMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_session_id: Option<String>,
}

/// Full session payload returned to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockSessionResponse {
    pub session:         DockSessionMeta,
    #[serde(default)]
    pub annotations:     Vec<DockAnnotation>,
    #[serde(default)]
    pub history:         Vec<DockHistoryEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_anchor: Option<String>,
    #[serde(default)]
    pub blocks:          Vec<DockBlock>,
    #[serde(default)]
    pub facts:           Vec<DockFact>,
}

/// Request payload for a user turn in the dock.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockTurnRequest {
    pub session_id:      String,
    pub content:         String,
    #[serde(default)]
    pub is_command:      bool,
    #[serde(default)]
    pub blocks:          Vec<DockBlock>,
    #[serde(default)]
    pub facts:           Vec<DockFact>,
    #[serde(default)]
    pub annotations:     Vec<DockAnnotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_anchor: Option<String>,
}

/// Response payload after processing a user turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockTurnResponse {
    pub session_id:      String,
    #[serde(default)]
    pub reply:           String,
    #[serde(default)]
    pub mutations:       Vec<DockMutation>,
    #[serde(default)]
    pub history:         Vec<DockHistoryEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_anchor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session:         Option<DockSessionMeta>,
    #[serde(default)]
    pub annotations:     Vec<DockAnnotation>,
    #[serde(default)]
    pub blocks:          Vec<DockBlock>,
    #[serde(default)]
    pub facts:           Vec<DockFact>,
}

/// Request to create a new session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockSessionCreateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id:    Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Request to update workspace-level state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockWorkspaceUpdateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_session_id: Option<String>,
}

/// A single entry in the session history timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockHistoryEntry {
    pub id:          String,
    #[serde(default)]
    pub anchor_name: String,
    #[serde(default)]
    pub timestamp:   String,
    #[serde(default)]
    pub label:       String,
    #[serde(default)]
    pub preview:     String,
    #[serde(default)]
    pub state:       serde_json::Value,
    #[serde(default)]
    pub is_selected: bool,
}
