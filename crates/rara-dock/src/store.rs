use std::path::{Path, PathBuf};

use snafu::ResultExt;
use tracing::debug;

use crate::{
    error::{self, DockError},
    models::{DockMutation, DockSessionDocument, DockSessionMeta, DockWorkspaceState, MutationOp},
};

/// File-based persistence for dock sessions and workspace state.
///
/// Storage layout:
/// ```text
/// {root}/
/// ├── workspace.json
/// └── sessions/
///     └── {session_id}/
///         └── document.json
/// ```
pub struct DockSessionStore {
    root: PathBuf,
}

impl DockSessionStore {
    /// Create a new store rooted at the given directory.
    pub fn new(root: PathBuf) -> Self { Self { root } }

    /// List all persisted sessions by reading session directories.
    pub fn list_sessions(&self) -> Result<Vec<DockSessionDocument>, DockError> {
        let sessions_dir = self.root.join("sessions");
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let entries = std::fs::read_dir(&sessions_dir).context(error::ListSessionsSnafu {
            path: sessions_dir.display().to_string(),
        })?;

        let mut docs = Vec::new();
        for entry in entries {
            let entry = entry.context(error::ListSessionsSnafu {
                path: sessions_dir.display().to_string(),
            })?;
            if entry.path().is_dir() {
                let doc_path = entry.path().join("document.json");
                if doc_path.exists() {
                    match self.read_document(&doc_path) {
                        Ok(doc) => docs.push(doc),
                        Err(e) => {
                            debug!(path = %doc_path.display(), error = %e, "Skipping corrupt session document");
                        }
                    }
                }
            }
        }

        // Sort by updated_at descending so the most recent session comes first.
        docs.sort_by(|a, b| b.session.updated_at.cmp(&a.session.updated_at));
        Ok(docs)
    }

    /// Load an existing session or create a new one if it does not exist.
    pub fn ensure_session(&self, id: &str) -> Result<DockSessionDocument, DockError> {
        Self::validate_session_id(id)?;
        let doc_path = self.session_document_path(id);
        if doc_path.exists() {
            self.read_document(&doc_path)
        } else {
            self.create_session(id, "Untitled")
        }
    }

    /// Create a brand-new session and persist it.
    ///
    /// Returns [`DockError::SessionAlreadyExists`] if a session with the given
    /// ID has already been persisted.
    pub fn create_session(&self, id: &str, title: &str) -> Result<DockSessionDocument, DockError> {
        Self::validate_session_id(id)?;

        let doc_path = self.session_document_path(id);
        if doc_path.exists() {
            return Err(DockError::SessionAlreadyExists { id: id.to_string() });
        }

        let now = now_millis();
        let doc = DockSessionDocument {
            session:     DockSessionMeta {
                id:              id.to_string(),
                title:           title.to_string(),
                preview:         String::new(),
                created_at:      now,
                updated_at:      now,
                selected_anchor: None,
            },
            blocks:      Vec::new(),
            annotations: Vec::new(),
            facts:       Vec::new(),
        };

        self.write_document(id, &doc)?;
        Ok(doc)
    }

    /// Apply a slice of mutations to a session document, persist, and return
    /// the updated document.
    pub fn apply_mutations(
        &self,
        session_id: &str,
        mutations: &[DockMutation],
    ) -> Result<DockSessionDocument, DockError> {
        let mut doc = self.ensure_session(session_id)?;

        for m in mutations {
            apply_mutation_to_document(&mut doc, m);
        }

        doc.session.updated_at = now_millis();
        self.write_document(session_id, &doc)?;
        Ok(doc)
    }

    /// Load workspace-level state (active session, etc.).
    pub fn load_workspace(&self) -> Result<DockWorkspaceState, DockError> {
        let path = self.root.join("workspace.json");
        if !path.exists() {
            return Ok(DockWorkspaceState {
                active_session_id: None,
            });
        }
        let data = std::fs::read_to_string(&path).context(error::ReadSnafu {
            path: path.display().to_string(),
        })?;
        serde_json::from_str(&data).context(error::DeserializeSnafu)
    }

    /// Persist workspace-level state.
    pub fn save_workspace(&self, state: &DockWorkspaceState) -> Result<(), DockError> {
        self.ensure_root_dir()?;
        let path = self.root.join("workspace.json");
        let data = serde_json::to_string_pretty(state).context(error::SerializeSnafu)?;
        std::fs::write(&path, data).context(error::WriteSnafu {
            path: path.display().to_string(),
        })
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn session_document_path(&self, id: &str) -> PathBuf {
        self.root.join("sessions").join(id).join("document.json")
    }

    fn read_document(&self, path: &Path) -> Result<DockSessionDocument, DockError> {
        let data = std::fs::read_to_string(path).context(error::ReadSnafu {
            path: path.display().to_string(),
        })?;
        serde_json::from_str(&data).context(error::DeserializeSnafu)
    }

    fn write_document(&self, session_id: &str, doc: &DockSessionDocument) -> Result<(), DockError> {
        let dir = self.root.join("sessions").join(session_id);
        std::fs::create_dir_all(&dir).context(error::CreateDirSnafu {
            path: dir.display().to_string(),
        })?;
        let path = dir.join("document.json");
        let data = serde_json::to_string_pretty(doc).context(error::SerializeSnafu)?;
        std::fs::write(&path, data).context(error::WriteSnafu {
            path: path.display().to_string(),
        })
    }

    fn ensure_root_dir(&self) -> Result<(), DockError> {
        std::fs::create_dir_all(&self.root).context(error::CreateDirSnafu {
            path: self.root.display().to_string(),
        })
    }

    /// Reject session IDs that could cause path traversal or filesystem issues.
    fn validate_session_id(id: &str) -> Result<(), DockError> {
        let is_safe = !id.is_empty()
            && !id.contains('/')
            && !id.contains('\\')
            && id != "."
            && id != ".."
            && id.len() <= 128;
        if is_safe {
            Ok(())
        } else {
            Err(DockError::InvalidSessionId { id: id.to_string() })
        }
    }
}

/// Apply a single mutation to a session document in memory.
fn apply_mutation_to_document(doc: &mut DockSessionDocument, mutation: &DockMutation) {
    match mutation.op {
        MutationOp::SessionUpsert => {
            // Update session metadata fields if a block carries title/preview info.
            // The caller typically sends updated DockSessionMeta via the session field
            // on the turn response; here we just bump updated_at.
            doc.session.updated_at = now_millis();
        }

        // -- Fact mutations ------------------------------------------------
        MutationOp::FactAdd => {
            if let Some(fact) = &mutation.fact {
                doc.facts.push(fact.clone());
            }
        }
        MutationOp::FactUpdate => {
            if let Some(fact) = &mutation.fact {
                if let Some(existing) = doc.facts.iter_mut().find(|f| f.id == fact.id) {
                    *existing = fact.clone();
                }
            }
        }
        MutationOp::FactRemove => {
            let remove_id = mutation
                .id
                .as_deref()
                .or(mutation.fact.as_ref().map(|f| f.id.as_str()));
            if let Some(id) = remove_id {
                doc.facts.retain(|f| f.id != id);
            }
        }

        // -- Annotation mutations ------------------------------------------
        MutationOp::AnnotationAdd => {
            if let Some(ann) = &mutation.annotation {
                doc.annotations.push(ann.clone());
            }
        }
        MutationOp::AnnotationUpdate => {
            if let Some(ann) = &mutation.annotation {
                if let Some(existing) = doc.annotations.iter_mut().find(|a| a.id == ann.id) {
                    existing.content = ann.content.clone();
                    if !ann.block_id.is_empty() {
                        existing.block_id = ann.block_id.clone();
                    }
                    if ann.anchor_y != 0.0 {
                        existing.anchor_y = ann.anchor_y;
                    }
                    if ann.selection.is_some() {
                        existing.selection = ann.selection.clone();
                    }
                    if ann.timestamp != 0 {
                        existing.timestamp = ann.timestamp;
                    }
                }
            }
        }
        MutationOp::AnnotationRemove => {
            let remove_id = mutation
                .id
                .as_deref()
                .or(mutation.annotation.as_ref().map(|a| a.id.as_str()));
            if let Some(id) = remove_id {
                doc.annotations.retain(|a| a.id != id);
            }
        }

        // -- Block mutations --------------------------------------------------
        MutationOp::BlockAdd => {
            if let Some(block) = &mutation.block {
                doc.blocks.push(block.clone());
            }
        }
        MutationOp::BlockUpdate => {
            if let Some(block) = &mutation.block {
                if let Some(existing) = doc.blocks.iter_mut().find(|b| b.id == block.id) {
                    *existing = block.clone();
                }
            }
        }
        MutationOp::BlockRemove => {
            let remove_id = mutation
                .id
                .as_deref()
                .or(mutation.block.as_ref().map(|b| b.id.as_str()));
            if let Some(id) = remove_id {
                doc.blocks.retain(|b| b.id != id);
            }
        }
    }
}

/// Current time in milliseconds since Unix epoch.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Actor, DockFact};

    #[test]
    fn test_create_and_ensure_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = DockSessionStore::new(dir.path().to_path_buf());

        let doc = store.create_session("sess-1", "My Session").unwrap();
        assert_eq!(doc.session.id, "sess-1");
        assert_eq!(doc.session.title, "My Session");

        let doc2 = store.ensure_session("sess-1").unwrap();
        assert_eq!(doc2.session.id, "sess-1");
    }

    #[test]
    fn test_list_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let store = DockSessionStore::new(dir.path().to_path_buf());

        store.create_session("a", "A").unwrap();
        store.create_session("b", "B").unwrap();

        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_apply_fact_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let store = DockSessionStore::new(dir.path().to_path_buf());
        store.create_session("s1", "Test").unwrap();

        let add = DockMutation {
            op:         MutationOp::FactAdd,
            actor:      Actor::Human,
            block:      None,
            fact:       Some(DockFact {
                id:      "f1".into(),
                content: "hello".into(),
                source:  Actor::Human,
            }),
            annotation: None,
            id:         None,
        };
        let doc = store.apply_mutations("s1", &[add]).unwrap();
        assert_eq!(doc.facts.len(), 1);
        assert_eq!(doc.facts[0].content, "hello");

        let remove = DockMutation {
            op:         MutationOp::FactRemove,
            actor:      Actor::Agent,
            block:      None,
            fact:       None,
            annotation: None,
            id:         Some("f1".into()),
        };
        let doc = store.apply_mutations("s1", &[remove]).unwrap();
        assert!(doc.facts.is_empty());
    }

    #[test]
    fn test_workspace_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = DockSessionStore::new(dir.path().to_path_buf());

        let state = store.load_workspace().unwrap();
        assert!(state.active_session_id.is_none());

        let state = DockWorkspaceState {
            active_session_id: Some("sess-1".into()),
        };
        store.save_workspace(&state).unwrap();

        let loaded = store.load_workspace().unwrap();
        assert_eq!(loaded.active_session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn test_invalid_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = DockSessionStore::new(dir.path().to_path_buf());

        assert!(store.create_session("../escape", "bad").is_err());
        assert!(store.create_session("", "bad").is_err());
        assert!(store.create_session("a/b", "bad").is_err());
    }
}
