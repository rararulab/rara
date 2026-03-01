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

//! Agent-to-agent pipes — streaming data channels between agent processes.
//!
//! Pipes provide a Unix-like mechanism for agents to stream data to each
//! other. A pipe is a unidirectional channel: one writer, one reader.
//!
//! Two flavours:
//! - **Anonymous pipes**: created by a parent agent, writer kept by parent,
//!   reader given to a target child agent.
//! - **Named pipes**: registered under a string name in the [`PipeRegistry`],
//!   allowing non-parent-child agents to rendezvous by name.
//!
//! # Architecture
//!
//! ```text
//! AgentA (writer) ──PipeWriter──► mpsc ──PipeReader──► AgentB (reader)
//! ```
//!
//! Under the hood each pipe is a `tokio::sync::mpsc` channel carrying
//! [`PipeMessage`] values. The [`PipeRegistry`] tracks ownership metadata
//! for introspection and lifecycle management.

use std::sync::Mutex;

use dashmap::DashMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::process::AgentId;

// ---------------------------------------------------------------------------
// PipeId
// ---------------------------------------------------------------------------

/// Unique identifier for a pipe (ULID string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipeId(pub String);

impl PipeId {
    /// Generate a new ULID-based pipe ID.
    pub fn new() -> Self { Self(ulid::Ulid::new().to_string()) }
}

impl Default for PipeId {
    fn default() -> Self { Self::new() }
}

impl std::fmt::Display for PipeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}

// ---------------------------------------------------------------------------
// PipeMessage
// ---------------------------------------------------------------------------

/// A single message transmitted through a pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeMessage {
    /// A data chunk (text payload).
    Data(String),
    /// An error message — the writer encountered a problem.
    Error(String),
    /// End-of-file marker — no more data will be sent.
    Eof,
}

// ---------------------------------------------------------------------------
// PipeWriter / PipeReader
// ---------------------------------------------------------------------------

/// Write end of a pipe.
///
/// Dropping the writer will cause the reader's [`PipeReader::recv`] to
/// eventually return `None`, signalling end-of-stream.
pub struct PipeWriter {
    pipe_id: PipeId,
    tx:      mpsc::Sender<PipeMessage>,
}

impl PipeWriter {
    /// The pipe this writer belongs to.
    pub fn pipe_id(&self) -> &PipeId { &self.pipe_id }

    /// Send a data message through the pipe.
    ///
    /// Returns `Err` if the reader has been dropped.
    pub async fn send(&self, data: String) -> Result<(), PipeSendError> {
        self.tx
            .send(PipeMessage::Data(data))
            .await
            .map_err(|_| PipeSendError)
    }

    /// Send an error message through the pipe.
    pub async fn send_error(&self, msg: String) -> Result<(), PipeSendError> {
        self.tx
            .send(PipeMessage::Error(msg))
            .await
            .map_err(|_| PipeSendError)
    }

    /// Send an explicit EOF and close the writer.
    ///
    /// After calling this the writer should be dropped.
    pub async fn send_eof(self) -> Result<(), PipeSendError> {
        self.tx
            .send(PipeMessage::Eof)
            .await
            .map_err(|_| PipeSendError)
    }
}

impl std::fmt::Debug for PipeWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipeWriter")
            .field("pipe_id", &self.pipe_id)
            .field("tx", &"<mpsc::Sender>")
            .finish()
    }
}

/// Read end of a pipe.
///
/// When the writer is dropped and all buffered messages are consumed,
/// [`recv`](Self::recv) returns `None`.
pub struct PipeReader {
    pipe_id: PipeId,
    rx:      mpsc::Receiver<PipeMessage>,
}

impl PipeReader {
    /// The pipe this reader belongs to.
    pub fn pipe_id(&self) -> &PipeId { &self.pipe_id }

    /// Receive the next message from the pipe.
    ///
    /// Returns `None` when the writer has been dropped and the buffer is
    /// exhausted (i.e., end-of-stream).
    pub async fn recv(&mut self) -> Option<PipeMessage> { self.rx.recv().await }
}

impl std::fmt::Debug for PipeReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipeReader")
            .field("pipe_id", &self.pipe_id)
            .field("rx", &"<mpsc::Receiver>")
            .finish()
    }
}

// ---------------------------------------------------------------------------
// PipeSendError
// ---------------------------------------------------------------------------

/// Error returned when writing to a pipe whose reader has been dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeSendError;

impl std::fmt::Display for PipeSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pipe closed: reader dropped")
    }
}

impl std::error::Error for PipeSendError {}

// ---------------------------------------------------------------------------
// pipe() — constructor
// ---------------------------------------------------------------------------

/// Create an anonymous pipe pair with the given buffer capacity.
///
/// Returns `(PipeWriter, PipeReader)` sharing the same [`PipeId`].
pub fn pipe(buffer: usize) -> (PipeWriter, PipeReader) {
    let (tx, rx) = mpsc::channel(buffer);
    let id = PipeId::new();
    (
        PipeWriter {
            pipe_id: id.clone(),
            tx,
        },
        PipeReader { pipe_id: id, rx },
    )
}

// ---------------------------------------------------------------------------
// PipeEntry — registry metadata
// ---------------------------------------------------------------------------

/// Metadata about a pipe tracked in the [`PipeRegistry`].
#[derive(Debug, Clone)]
pub struct PipeEntry {
    /// The agent that created (owns) this pipe.
    pub owner:      AgentId,
    /// The agent connected as reader (if any).
    pub reader:     Option<AgentId>,
    /// When the pipe was created.
    pub created_at: Timestamp,
}

// ---------------------------------------------------------------------------
// PipeRegistry
// ---------------------------------------------------------------------------

/// Central registry tracking active pipes and their ownership.
///
/// Supports both anonymous pipes (tracked by [`PipeId`]) and named pipes
/// (tracked by an additional string key).
///
/// Named pipes support a "parking" mechanism: the creator parks the reader
/// end in the registry, and a connecting agent retrieves it via
/// [`take_parked_reader`](Self::take_parked_reader).
pub struct PipeRegistry {
    /// All active pipes keyed by PipeId.
    pipes: DashMap<PipeId, PipeEntry>,
    /// Named pipe index: name -> PipeId.
    named: DashMap<String, PipeId>,
    /// Parked readers for named pipes (take-once via Mutex<Option>).
    parked_readers: DashMap<PipeId, Mutex<Option<PipeReader>>>,
}

impl PipeRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            pipes:          DashMap::new(),
            named:          DashMap::new(),
            parked_readers: DashMap::new(),
        }
    }

    /// Register a pipe entry.
    pub fn register(&self, pipe_id: PipeId, entry: PipeEntry) {
        self.pipes.insert(pipe_id, entry);
    }

    /// Register a named pipe (also adds to the pipe table).
    pub fn register_named(&self, name: String, pipe_id: PipeId, entry: PipeEntry) {
        self.pipes.insert(pipe_id.clone(), entry);
        self.named.insert(name, pipe_id);
    }

    /// Park a reader end for a named pipe, so a connecting agent can take it.
    pub fn park_reader(&self, pipe_id: PipeId, reader: PipeReader) {
        self.parked_readers
            .insert(pipe_id, Mutex::new(Some(reader)));
    }

    /// Take the parked reader for a named pipe (one-shot).
    ///
    /// Returns `None` if no reader was parked or it has already been taken.
    pub fn take_parked_reader(&self, pipe_id: &PipeId) -> Option<PipeReader> {
        self.parked_readers
            .get(pipe_id)
            .and_then(|slot| slot.value().lock().ok()?.take())
    }

    /// Look up the PipeId for a named pipe.
    pub fn resolve_name(&self, name: &str) -> Option<PipeId> {
        self.named.get(name).map(|r| r.value().clone())
    }

    /// Set the reader agent on a pipe entry.
    pub fn set_reader(&self, pipe_id: &PipeId, reader: AgentId) -> bool {
        if let Some(mut entry) = self.pipes.get_mut(pipe_id) {
            entry.reader = Some(reader);
            true
        } else {
            false
        }
    }

    /// Get metadata for a pipe.
    pub fn get(&self, pipe_id: &PipeId) -> Option<PipeEntry> {
        self.pipes.get(pipe_id).map(|r| r.value().clone())
    }

    /// Remove a pipe from the registry (including its named entry and parked
    /// reader if any).
    pub fn remove(&self, pipe_id: &PipeId) {
        self.pipes.remove(pipe_id);
        self.parked_readers.remove(pipe_id);
        // Clean up named reference if any
        self.named.retain(|_, v| v != pipe_id);
    }

    /// List all pipes owned by an agent.
    pub fn pipes_by_owner(&self, owner: AgentId) -> Vec<PipeId> {
        self.pipes
            .iter()
            .filter(|entry| entry.value().owner == owner)
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Count of active pipes.
    pub fn len(&self) -> usize { self.pipes.len() }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool { self.pipes.is_empty() }

    /// Count of named pipes.
    pub fn named_count(&self) -> usize { self.named.len() }
}

impl Default for PipeRegistry {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pipe_send_recv() {
        let (writer, mut reader) = pipe(16);

        writer.send("hello".to_string()).await.unwrap();
        writer.send("world".to_string()).await.unwrap();

        let msg = reader.recv().await.unwrap();
        assert_eq!(msg, PipeMessage::Data("hello".to_string()));

        let msg = reader.recv().await.unwrap();
        assert_eq!(msg, PipeMessage::Data("world".to_string()));
    }

    #[tokio::test]
    async fn test_pipe_eof() {
        let (writer, mut reader) = pipe(16);

        writer.send("data".to_string()).await.unwrap();
        writer.send_eof().await.unwrap();

        let msg = reader.recv().await.unwrap();
        assert_eq!(msg, PipeMessage::Data("data".to_string()));

        let msg = reader.recv().await.unwrap();
        assert_eq!(msg, PipeMessage::Eof);

        // After EOF and writer dropped, recv returns None
        let msg = reader.recv().await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_pipe_writer_drop_closes_reader() {
        let (writer, mut reader) = pipe(16);

        writer.send("before drop".to_string()).await.unwrap();
        drop(writer);

        // Should still receive buffered message
        let msg = reader.recv().await.unwrap();
        assert_eq!(msg, PipeMessage::Data("before drop".to_string()));

        // Next recv returns None (channel closed)
        let msg = reader.recv().await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_pipe_error_message() {
        let (writer, mut reader) = pipe(16);

        writer.send_error("something went wrong".to_string()).await.unwrap();

        let msg = reader.recv().await.unwrap();
        assert_eq!(msg, PipeMessage::Error("something went wrong".to_string()));
    }

    #[tokio::test]
    async fn test_pipe_reader_drop_writer_gets_error() {
        let (writer, reader) = pipe(16);

        drop(reader);

        let result = writer.send("orphaned".to_string()).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), PipeSendError);
    }

    #[tokio::test]
    async fn test_pipe_multiple_messages_flow() {
        let (writer, mut reader) = pipe(16);

        let n = 100;

        // Writer and reader must run concurrently (buffer < n).
        let writer_task = tokio::spawn(async move {
            for i in 0..n {
                writer.send(format!("msg-{i}")).await.unwrap();
            }
        });

        let reader_task = tokio::spawn(async move {
            for i in 0..n {
                let msg = reader.recv().await.unwrap();
                assert_eq!(msg, PipeMessage::Data(format!("msg-{i}")));
            }
        });

        writer_task.await.unwrap();
        reader_task.await.unwrap();
    }

    #[test]
    fn test_pipe_id_display() {
        let id = PipeId::new();
        let display = id.to_string();
        assert!(!display.is_empty());
        // ULID is 26 chars
        assert_eq!(display.len(), 26);
    }

    #[test]
    fn test_pipe_writer_debug() {
        let (writer, _reader) = pipe(1);
        let debug = format!("{:?}", writer);
        assert!(debug.contains("PipeWriter"));
        assert!(debug.contains("pipe_id"));
    }

    #[test]
    fn test_pipe_reader_debug() {
        let (_writer, reader) = pipe(1);
        let debug = format!("{:?}", reader);
        assert!(debug.contains("PipeReader"));
        assert!(debug.contains("pipe_id"));
    }

    #[test]
    fn test_pipe_send_error_display() {
        let err = PipeSendError;
        assert_eq!(err.to_string(), "pipe closed: reader dropped");
    }

    // -- PipeRegistry tests ---------------------------------------------------

    #[test]
    fn test_registry_register_and_get() {
        let registry = PipeRegistry::new();
        let pipe_id = PipeId::new();
        let owner = AgentId::new();

        registry.register(
            pipe_id.clone(),
            PipeEntry {
                owner,
                reader:     None,
                created_at: Timestamp::now(),
            },
        );

        let entry = registry.get(&pipe_id).unwrap();
        assert_eq!(entry.owner, owner);
        assert!(entry.reader.is_none());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_registry_named_pipe() {
        let registry = PipeRegistry::new();
        let pipe_id = PipeId::new();
        let owner = AgentId::new();

        registry.register_named(
            "my-pipe".to_string(),
            pipe_id.clone(),
            PipeEntry {
                owner,
                reader:     None,
                created_at: Timestamp::now(),
            },
        );

        // Resolve by name
        let resolved = registry.resolve_name("my-pipe").unwrap();
        assert_eq!(resolved, pipe_id);
        assert_eq!(registry.named_count(), 1);

        // Unknown name
        assert!(registry.resolve_name("nonexistent").is_none());
    }

    #[test]
    fn test_registry_set_reader() {
        let registry = PipeRegistry::new();
        let pipe_id = PipeId::new();
        let owner = AgentId::new();
        let reader_id = AgentId::new();

        registry.register(
            pipe_id.clone(),
            PipeEntry {
                owner,
                reader:     None,
                created_at: Timestamp::now(),
            },
        );

        assert!(registry.set_reader(&pipe_id, reader_id));

        let entry = registry.get(&pipe_id).unwrap();
        assert_eq!(entry.reader, Some(reader_id));
    }

    #[test]
    fn test_registry_set_reader_nonexistent() {
        let registry = PipeRegistry::new();
        let pipe_id = PipeId::new();
        let reader_id = AgentId::new();

        assert!(!registry.set_reader(&pipe_id, reader_id));
    }

    #[test]
    fn test_registry_remove() {
        let registry = PipeRegistry::new();
        let pipe_id = PipeId::new();
        let owner = AgentId::new();

        registry.register_named(
            "removable".to_string(),
            pipe_id.clone(),
            PipeEntry {
                owner,
                reader:     None,
                created_at: Timestamp::now(),
            },
        );

        assert_eq!(registry.len(), 1);
        assert_eq!(registry.named_count(), 1);

        registry.remove(&pipe_id);

        assert_eq!(registry.len(), 0);
        assert_eq!(registry.named_count(), 0);
        assert!(registry.resolve_name("removable").is_none());
    }

    #[test]
    fn test_registry_pipes_by_owner() {
        let registry = PipeRegistry::new();
        let owner_a = AgentId::new();
        let owner_b = AgentId::new();

        for _ in 0..3 {
            registry.register(
                PipeId::new(),
                PipeEntry {
                    owner:      owner_a,
                    reader:     None,
                    created_at: Timestamp::now(),
                },
            );
        }
        registry.register(
            PipeId::new(),
            PipeEntry {
                owner:      owner_b,
                reader:     None,
                created_at: Timestamp::now(),
            },
        );

        let a_pipes = registry.pipes_by_owner(owner_a);
        assert_eq!(a_pipes.len(), 3);

        let b_pipes = registry.pipes_by_owner(owner_b);
        assert_eq!(b_pipes.len(), 1);
    }

    #[test]
    fn test_registry_empty() {
        let registry = PipeRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert_eq!(registry.named_count(), 0);
    }

    #[tokio::test]
    async fn test_pipe_between_two_agents_simulated() {
        // Simulate two agents communicating through a pipe
        let (writer, mut reader) = pipe(16);
        let registry = PipeRegistry::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        // Agent A creates the pipe
        registry.register(
            writer.pipe_id().clone(),
            PipeEntry {
                owner:      agent_a,
                reader:     Some(agent_b),
                created_at: Timestamp::now(),
            },
        );

        // Agent A writes data
        let writer_task = tokio::spawn(async move {
            writer.send("line 1".to_string()).await.unwrap();
            writer.send("line 2".to_string()).await.unwrap();
            writer.send("line 3".to_string()).await.unwrap();
            writer.send_eof().await.unwrap();
        });

        // Agent B reads data
        let reader_task = tokio::spawn(async move {
            let mut collected = Vec::new();
            while let Some(msg) = reader.recv().await {
                match msg {
                    PipeMessage::Data(s) => collected.push(s),
                    PipeMessage::Eof => break,
                    PipeMessage::Error(e) => panic!("unexpected error: {e}"),
                }
            }
            collected
        });

        writer_task.await.unwrap();
        let result = reader_task.await.unwrap();
        assert_eq!(result, vec!["line 1", "line 2", "line 3"]);
    }

    #[tokio::test]
    async fn test_named_pipe_rendezvous() {
        // Simulate two unrelated agents using a named pipe
        let registry = PipeRegistry::new();
        let producer = AgentId::new();
        let consumer = AgentId::new();

        // Producer creates a named pipe
        let (writer, reader) = pipe(16);
        let pipe_id = writer.pipe_id().clone();
        registry.register_named(
            "data-feed".to_string(),
            pipe_id.clone(),
            PipeEntry {
                owner:      producer,
                reader:     None,
                created_at: Timestamp::now(),
            },
        );

        // Consumer discovers the pipe by name
        let resolved_id = registry.resolve_name("data-feed").unwrap();
        assert_eq!(resolved_id, pipe_id);
        registry.set_reader(&resolved_id, consumer);

        // Verify metadata
        let entry = registry.get(&resolved_id).unwrap();
        assert_eq!(entry.owner, producer);
        assert_eq!(entry.reader, Some(consumer));

        // Data flows
        let mut reader = reader;
        writer.send("named-data".to_string()).await.unwrap();
        let msg = reader.recv().await.unwrap();
        assert_eq!(msg, PipeMessage::Data("named-data".to_string()));
    }
}
