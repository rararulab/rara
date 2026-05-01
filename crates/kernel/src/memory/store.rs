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

//! JSONL-backed persistence layer for the tape subsystem.
//!
//! [`FileTapeStore`] is the async-facing handle that dispatches all file I/O to
//! a dedicated blocking thread (`rara-tape-io`).  Internally, each tape is
//! managed by a `TapeFile` that maintains an in-memory entry cache and a
//! byte-offset cursor for incremental reads.  Fork, merge, and discard
//! operations are implemented here as file-level copy / append / delete.

use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use rustix::{fs as rustix_fs, io as rustix_io};
use serde_json::Value;
use snafu::ResultExt;
use tokio::sync::oneshot;
use urlencoding::{decode, encode};

use super::{TAPE_FILE_SUFFIX, TapEntry, TapEntryKind, TapError, TapResult};

/// Per-append result returned by [`FileTapeStore::append`].
///
/// Issue #2025: the session-index update path needs the byte offset of
/// the JSONL line just written (to record `AnchorRef.byte_offset`) and
/// the post-append entry count (so it can mirror `TapeInfo.entries`
/// without a follow-up O(N) `info()` call). Both values are computed
/// inside the tape I/O worker for free, so we surface them rather than
/// having every caller pay an extra round trip.
#[derive(Debug, Clone)]
pub struct AppendOutcome {
    /// The persisted entry, with its assigned id.
    pub entry:               TapEntry,
    /// Byte offset where this entry's JSONL line begins in the tape file.
    pub byte_offset:         u64,
    /// Total number of entries on the tape after this append.
    pub total_entries_after: i64,
}

impl AppendOutcome {
    /// Convenience accessor for the assigned entry id.
    pub fn entry_id(&self) -> u64 { self.entry.id }
}

// Pluggable JSONL codec (issue #2007). Declared from `store.rs` rather
// than `mod.rs` so the PoC scope stays inside the boundaries listed in
// the spec. When `feature = "zig-codec"` is on, `encode_entry` round-
// trips through the Zig static lib; otherwise both helpers are thin
// shims around `serde_json`.
#[path = "codec.rs"]
mod codec;

type Job = Box<dyn FnOnce(&mut WorkerState) + Send + 'static>;

/// In-memory secondary index over a tape's cached entries.
///
/// The index lets `TapeFile` answer kind-filter and anchor-name queries in
/// O(1)/O(k) instead of O(n) linear scans across `read_entries`.  All maps
/// store **offsets into `read_entries`** rather than entry IDs so the lookup
/// remains O(1) even after the cache is partially invalidated and rebuilt.
///
/// # Consistency invariant
///
/// `TapeIndex` MUST stay in lockstep with `TapeFile::read_entries`:
///
/// - Every `push` to `read_entries` is paired with `TapeIndex::insert_entry`.
/// - Every `clear`/`reset_cache` is paired with `TapeIndex::clear`.
///
/// All mutation paths (`append_many`, `ensure_cached`, `copy_to`, `copy_from`,
/// `reset_cache`) funnel through these helpers — see `TapeFile::push_entry`
/// and `TapeFile::reset_cache` for the single points of update.
#[derive(Debug, Default)]
struct TapeIndex {
    /// Maps an entry ID to its offset in `read_entries`.
    by_id:          HashMap<u64, usize>,
    /// Maps each entry kind to the offsets of entries with that kind, in
    /// append order.
    by_kind:        HashMap<TapEntryKind, Vec<usize>>,
    /// Maps an anchor name to the offsets of every anchor entry with that
    /// name, in append order.  Most lookups want the most recent occurrence,
    /// so callers consult `.last()`.
    anchor_by_name: HashMap<String, Vec<usize>>,
}

impl TapeIndex {
    /// Drop every indexed entry.
    fn clear(&mut self) {
        self.by_id.clear();
        self.by_kind.clear();
        self.anchor_by_name.clear();
    }

    /// Insert one entry already pushed into the parent `read_entries` vec.
    /// `offset` must be the index of `entry` inside `read_entries`.
    fn insert_entry(&mut self, offset: usize, entry: &TapEntry) {
        self.by_id.insert(entry.id, offset);
        self.by_kind.entry(entry.kind).or_default().push(offset);
        if entry.kind == TapEntryKind::Anchor
            && let Some(name) = entry.payload.get("name").and_then(Value::as_str)
        {
            self.anchor_by_name
                .entry(name.to_owned())
                .or_default()
                .push(offset);
        }
    }
}

/// Mutable helper for one on-disk tape file.
///
/// `TapeFile` keeps a small in-memory cache so repeated reads can resume from
/// the last known byte offset instead of reparsing the entire file every time.
#[derive(Debug)]
struct TapeFile {
    /// On-disk location of the JSONL tape file.
    path:          PathBuf,
    /// First ID reserved for fork-local entries after cloning from a parent.
    fork_start_id: Option<u64>,
    /// Fully decoded entries cached in append order.
    read_entries:  Vec<TapEntry>,
    /// Secondary index built from `read_entries` and kept in sync on every
    /// mutation path.  See [`TapeIndex`] for the consistency invariant.
    index:         TapeIndex,
    /// Number of bytes already consumed from the file.
    read_offset:   u64,
    /// Trailing bytes that do not yet end in a full JSONL record.
    tail_bytes:    Vec<u8>,
    /// Lazily opened descriptor reused for positional reads and writes.
    file:          Option<File>,
}

impl TapeFile {
    /// Create a new file helper for the target path.
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            fork_start_id: None,
            read_entries: Vec::new(),
            index: TapeIndex::default(),
            read_offset: 0,
            tail_bytes: Vec::new(),
            file: None,
        }
    }

    /// Append `entry` to `read_entries` and update the secondary index in
    /// the same step.  This is the **only** allowed way to grow the cache so
    /// the index can never drift from `read_entries`.
    fn push_entry(&mut self, entry: TapEntry) {
        let offset = self.read_entries.len();
        self.index.insert_entry(offset, &entry);
        self.read_entries.push(entry);
    }

    /// Copy entries into `target`.  If `at_entry_id` is `Some`, only copy
    /// entries with `id >= fork_point` (partial fork).  Otherwise copy
    /// everything (full fork).
    fn copy_to(&mut self, target: &mut Self, at_entry_id: Option<u64>) -> TapResult<()> {
        self.ensure_cached()?;
        target.close_file();

        match at_entry_id {
            Some(fork_point) => {
                // Partial fork: only write entries from fork_point onward.
                let entries: Vec<TapEntry> = self
                    .read_entries
                    .iter()
                    .filter(|e| e.id >= fork_point)
                    .cloned()
                    .collect();
                target.append_many(entries)?;
                target.fork_start_id = Some(target.next_id());
            }
            None => {
                // Full fork: copy entire file (original behavior).
                self.close_file();
                if self.path.exists() {
                    fs::copy(&self.path, &target.path).context(super::error::IoSnafu)?;
                } else {
                    File::create(&target.path).context(super::error::IoSnafu)?;
                }
                self.ensure_cached()?;
                target.reset_cache();
                for entry in &self.read_entries {
                    target.push_entry(entry.clone());
                }
                target.read_offset = self.read_offset;
                target.fork_start_id = Some(self.next_id());
            }
        }
        Ok(())
    }

    /// Append entries from `source` that were created after the fork point.
    fn copy_from(&mut self, source: &mut Self) -> TapResult<()> {
        source.ensure_cached()?;
        let entries = source
            .read_entries
            .iter()
            .filter(|entry| entry.id >= source.fork_start_id.unwrap_or(1))
            .cloned()
            .collect::<Vec<_>>();
        self.append_many(entries)?;
        Ok(())
    }

    /// Return the next ID that should be assigned if another entry is written.
    fn next_id(&self) -> u64 {
        self.read_entries
            .last()
            .map_or(1, |entry| entry.id.saturating_add(1))
    }

    /// Drop the cached entries, secondary index, and byte offset.
    fn reset_cache(&mut self) {
        self.read_entries.clear();
        self.index.clear();
        self.read_offset = 0;
        self.tail_bytes.clear();
    }

    /// Remove the underlying file and clear the in-memory cache.
    fn reset(&mut self) -> TapResult<()> {
        self.close_file();
        if self.path.exists() {
            fs::remove_file(&self.path).context(super::error::IoSnafu)?;
        }
        self.reset_cache();
        Ok(())
    }

    /// Sync any new bytes from disk into the in-memory cache without cloning.
    fn ensure_cached(&mut self) -> TapResult<()> {
        if !self.path.exists() {
            self.close_file();
            self.reset_cache();
            return Ok(());
        }

        let file_size = self.file_len()?;
        if file_size < self.read_offset {
            self.reset_cache();
        }

        let remaining = file_size.saturating_sub(self.read_offset);
        if remaining == 0 {
            return Ok(());
        }

        let mut buffer = vec![0_u8; remaining as usize];
        let bytes_read = self.read_at(&mut buffer, self.read_offset)?;
        buffer.truncate(bytes_read);

        let mut chunk = std::mem::take(&mut self.tail_bytes);
        chunk.extend_from_slice(&buffer);

        let Some(last_newline) = chunk.iter().rposition(|byte| *byte == b'\n') else {
            self.tail_bytes = chunk;
            self.read_offset = self.read_offset.saturating_add(bytes_read as u64);
            return Ok(());
        };

        let trailing = chunk.split_off(last_newline + 1);
        self.tail_bytes = trailing;

        for line in chunk.split(|byte| *byte == b'\n') {
            let trimmed = trim_ascii(line);
            if trimmed.is_empty() {
                continue;
            }
            let entry = codec::decode_entry(trimmed)?;
            self.push_entry(entry);
        }

        self.read_offset = self.read_offset.saturating_add(bytes_read as u64);
        Ok(())
    }

    /// Read any new entries from disk into the cache, then return the full
    /// cached entry list.
    fn read(&mut self) -> TapResult<Vec<TapEntry>> {
        self.ensure_cached()?;
        Ok(self.read_entries.clone())
    }

    /// Return the entry ID of the most recent `Anchor` entry, if any.
    /// O(1) via the secondary index instead of an O(n) reverse linear scan.
    fn last_anchor_id(&mut self) -> TapResult<Option<u64>> {
        self.ensure_cached()?;
        Ok(self
            .index
            .by_kind
            .get(&TapEntryKind::Anchor)
            .and_then(|offsets| offsets.last())
            .map(|&offset| self.read_entries[offset].id))
    }

    /// Return the entry ID of the most recent `Anchor` whose payload `name`
    /// matches `anchor_name`.  O(1) via the secondary index.
    fn last_anchor_id_by_name(&mut self, anchor_name: &str) -> TapResult<Option<u64>> {
        self.ensure_cached()?;
        Ok(self
            .index
            .anchor_by_name
            .get(anchor_name)
            .and_then(|offsets| offsets.last())
            .map(|&offset| self.read_entries[offset].id))
    }

    /// Clone every cached entry whose kind is `kind`, in append order.
    /// O(k) in the number of matching entries via the secondary index instead
    /// of O(n) over the whole tape.
    fn entries_by_kind(&mut self, kind: TapEntryKind) -> TapResult<Vec<TapEntry>> {
        self.ensure_cached()?;
        Ok(self
            .index
            .by_kind
            .get(&kind)
            .map(|offsets| {
                offsets
                    .iter()
                    .map(|&offset| self.read_entries[offset].clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Append one entry, assigning its persisted ID first.
    fn append(&mut self, entry: TapEntry) -> TapResult<AppendOutcome> {
        let mut outcomes = self.append_many(vec![entry])?;
        Ok(outcomes.remove(0))
    }

    /// Append multiple entries in order, assigning new IDs during persistence.
    ///
    /// Each returned [`AppendOutcome`] carries the per-entry byte offset
    /// at which that entry's JSONL line begins in the tape file. Callers
    /// (notably the session-index update path in `TapeService`) rely on
    /// this for `AnchorRef.byte_offset`.
    fn append_many(&mut self, entries: Vec<TapEntry>) -> TapResult<Vec<AppendOutcome>> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        self.ensure_cached()?;
        let mut next_id = self.next_id();
        let mut offset = self.read_offset;
        let mut stored = Vec::with_capacity(entries.len());
        let mut byte_offsets = Vec::with_capacity(entries.len());
        let mut encoded_batch = Vec::new();

        for mut entry in entries {
            entry.id = next_id;
            // Record the start offset of *this* entry's JSONL line before
            // we extend the batch buffer. `byte_offset` is therefore the
            // file position at which `entry`'s line will live once
            // `pwrite` flushes the batch.
            byte_offsets.push(offset.saturating_add(encoded_batch.len() as u64));
            let mut encoded = codec::encode_entry(&entry)?;
            encoded.push(b'\n');
            encoded_batch.extend_from_slice(&encoded);
            stored.push(entry);
            next_id = next_id.saturating_add(1);
        }

        self.write_all_at(&encoded_batch, offset)?;
        self.sync_file()?;
        offset = offset.saturating_add(encoded_batch.len() as u64);
        for entry in &stored {
            self.push_entry(entry.clone());
        }
        self.read_offset = offset;

        // `read_entries.len()` is the rolling total — see Decision 3 in
        // specs/issue-2025-session-index-tape-derived-state.spec.md and
        // the parent's "TotalEntries source" directive. Maintained
        // consistently across every lifecycle path (`copy_to`, `copy_from`,
        // `reset_cache`) by funnelling all mutations through `push_entry`
        // / `read_entries.clear()`.
        let total_after_full = self.read_entries.len() as i64;
        let new_count = stored.len() as i64;
        Ok(stored
            .into_iter()
            .zip(byte_offsets)
            .enumerate()
            .map(|(idx, (entry, byte_offset))| AppendOutcome {
                entry,
                byte_offset,
                // The Nth entry in the batch has a `total_entries_after`
                // of `total_after_full - (batch_len - 1 - idx)`.
                total_entries_after: total_after_full - new_count + 1 + idx as i64,
            })
            .collect())
    }

    /// Move the active tape file into a timestamped `.bak` archive file.
    fn archive(&mut self) -> TapResult<Option<PathBuf>> {
        if !self.path.exists() {
            self.close_file();
            self.reset_cache();
            return Ok(None);
        }
        self.close_file();

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|source| TapError::State {
                message: source.to_string(),
            })?
            .as_secs();
        let archive_path = PathBuf::from(format!("{}.{stamp}.bak", self.path.display()));
        fs::rename(&self.path, &archive_path).context(super::error::IoSnafu)?;
        self.reset_cache();
        Ok(Some(archive_path))
    }

    /// Clear cached state when the file disappears outside this process.
    fn clear_missing(&mut self) {
        self.close_file();
        self.reset_cache();
    }

    /// Return the current file length from the open descriptor.
    fn file_len(&mut self) -> TapResult<u64> {
        let file = self.ensure_file(false)?;
        let stat = rustix_fs::fstat(&*file).map_err(rustix_error)?;
        Ok(stat.st_size as u64)
    }

    /// Fill `buffer` using positional reads starting at `offset`.
    fn read_at(&mut self, buffer: &mut [u8], offset: u64) -> TapResult<usize> {
        let file = self.ensure_file(false)?;
        let mut filled = 0;
        while filled < buffer.len() {
            let read = rustix_io::pread(&*file, &mut buffer[filled..], offset + filled as u64)
                .map_err(rustix_error)?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        Ok(filled)
    }

    /// Write the entire buffer using positional writes, retrying partial
    /// writes.
    fn write_all_at(&mut self, buffer: &[u8], offset: u64) -> TapResult<()> {
        let file = self.ensure_file(true)?;
        let mut written = 0;
        while written < buffer.len() {
            let bytes = rustix_io::pwrite(&*file, &buffer[written..], offset + written as u64)
                .map_err(rustix_error)?;
            if bytes == 0 {
                return Err(TapError::Io {
                    source: std::io::Error::new(
                        ErrorKind::WriteZero,
                        "pwrite wrote zero bytes for append-only tape",
                    ),
                });
            }
            written += bytes;
        }
        Ok(())
    }

    /// Flush file contents to stable storage.
    fn sync_file(&mut self) -> TapResult<()> {
        let file = self.ensure_file(false)?;
        rustix_fs::fsync(&*file).map_err(rustix_error)?;
        Ok(())
    }

    /// Open the file descriptor on demand, creating the file if requested.
    fn ensure_file(&mut self, create: bool) -> TapResult<&mut File> {
        if self.file.is_none() {
            let mut options = OpenOptions::new();
            options.read(true).write(true);
            if create {
                options.create(true);
            }
            let file = options.open(&self.path).context(super::error::IoSnafu)?;
            self.file = Some(file);
        }

        self.file.as_mut().ok_or_else(|| TapError::State {
            message: "tape file handle missing after open".to_owned(),
        })
    }

    /// Drop the cached descriptor so future operations reopen the file.
    fn close_file(&mut self) { self.file = None; }
}

/// State owned exclusively by the blocking tape I/O thread.
///
/// Keeping the cache map here ensures all filesystem mutation and cache updates
/// happen on one thread, so `TapeFile` itself does not need interior locking.
#[derive(Debug)]
struct WorkerState {
    tape_root:      PathBuf,
    workspace_hash: String,
    tape_files:     HashMap<String, TapeFile>,
}

impl WorkerState {
    /// Build worker state for one workspace-local tape namespace.
    fn new(home: &Path, workspace_path: &Path) -> TapResult<Self> {
        let tape_root = home.join("tapes");
        fs::create_dir_all(&tape_root).context(super::error::IoSnafu)?;
        Ok(Self {
            tape_root,
            workspace_hash: md5_hex(
                &workspace_path
                    .canonicalize()
                    .context(super::error::IoSnafu)?,
            ),
            tape_files: HashMap::new(),
        })
    }

    /// Scan the tape directory and return active tape names for this workspace.
    fn list_tapes(&mut self) -> TapResult<Vec<String>> {
        let prefix = format!("{}__", self.workspace_hash);
        let mut tapes = Vec::new();
        for path in fs::read_dir(&self.tape_root).context(super::error::IoSnafu)? {
            let path = path.context(super::error::IoSnafu)?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with(&prefix) || !name.ends_with(TAPE_FILE_SUFFIX) {
                continue;
            }

            let encoded = name
                .trim_start_matches(&prefix)
                .trim_end_matches(TAPE_FILE_SUFFIX);
            if encoded.is_empty() || encoded.contains("__") {
                continue;
            }
            let decoded = decode(encoded)
                .context(super::error::UrlDecodeSnafu)?
                .into_owned();
            tapes.push(decoded);
        }
        tapes.sort();
        tapes.dedup();
        Ok(tapes)
    }

    /// Create a fork by cloning the source file and cache state into a new
    /// tape.  When `at_entry_id` is `Some`, only entries from that ID onward
    /// are included in the fork (partial fork).
    fn fork(&mut self, source: &str, at_entry_id: Option<u64>) -> TapResult<String> {
        let fork_name = format!("{source}__{:08x}", unique_suffix());
        let mut source_file = self.take_tape_file(source);
        let mut target_file = self.take_tape_file(&fork_name);
        source_file.copy_to(&mut target_file, at_entry_id)?;
        self.tape_files.insert(source.to_owned(), source_file);
        self.tape_files.insert(fork_name.clone(), target_file);
        Ok(fork_name)
    }

    /// Merge fork-local entries back into `target`, then delete the fork file.
    fn merge(&mut self, source: &str, target: &str) -> TapResult<()> {
        let mut source_file = self.take_tape_file(source);
        let mut target_file = self.take_tape_file(target);
        target_file.copy_from(&mut source_file)?;
        if source_file.path.exists() {
            source_file.close_file();
            fs::remove_file(&source_file.path).context(super::error::IoSnafu)?;
        }
        self.tape_files.insert(target.to_owned(), target_file);
        Ok(())
    }

    /// Delete a fork tape file and remove it from the cache without merging.
    fn discard(&mut self, fork_tape: &str) -> TapResult<()> {
        let mut file = self.take_tape_file(fork_tape);
        file.close_file();
        if file.path.exists() {
            fs::remove_file(&file.path).context(super::error::IoSnafu)?;
        }
        Ok(())
    }

    /// Remove one tape and clear any cached state for it.
    fn reset(&mut self, tape: &str) -> TapResult<()> {
        let path = self.tape_path(tape);
        self.tape_files
            .entry(tape.to_owned())
            .or_insert_with(|| TapeFile::new(path))
            .reset()
    }

    /// Read one tape if it exists, invalidating stale cache state if it does
    /// not.
    fn read(&mut self, tape: &str) -> TapResult<Option<Vec<TapEntry>>> {
        let path = self.tape_path(tape);
        if !path.exists() {
            if let Some(file) = self.tape_files.get_mut(tape) {
                file.clear_missing();
            }
            return Ok(None);
        }

        self.tape_files
            .entry(tape.to_owned())
            .or_insert_with(|| TapeFile::new(path))
            .read()
            .map(Some)
    }

    /// O(1) lookup for the most recent anchor ID on `tape`, or `None` if the
    /// tape has no anchors (or does not yet exist).
    fn last_anchor_id(&mut self, tape: &str) -> TapResult<Option<u64>> {
        let path = self.tape_path(tape);
        if !path.exists() {
            return Ok(None);
        }
        self.tape_files
            .entry(tape.to_owned())
            .or_insert_with(|| TapeFile::new(path))
            .last_anchor_id()
    }

    /// O(1) lookup for the most recent anchor ID matching `anchor_name`,
    /// or `None` if no such anchor exists on this tape.
    fn last_anchor_id_by_name(&mut self, tape: &str, anchor_name: &str) -> TapResult<Option<u64>> {
        let path = self.tape_path(tape);
        if !path.exists() {
            return Ok(None);
        }
        self.tape_files
            .entry(tape.to_owned())
            .or_insert_with(|| TapeFile::new(path))
            .last_anchor_id_by_name(anchor_name)
    }

    /// Read every entry whose kind matches `kind`, using the secondary
    /// index to skip linear filtering.
    fn entries_by_kind(
        &mut self,
        tape: &str,
        kind: super::TapEntryKind,
    ) -> TapResult<Vec<TapEntry>> {
        let path = self.tape_path(tape);
        if !path.exists() {
            return Ok(Vec::new());
        }
        self.tape_files
            .entry(tape.to_owned())
            .or_insert_with(|| TapeFile::new(path))
            .entries_by_kind(kind)
    }

    /// Persist one new entry to the requested tape.
    fn append(
        &mut self,
        tape: &str,
        kind: super::TapEntryKind,
        payload: serde_json::Value,
        metadata: Option<serde_json::Value>,
    ) -> TapResult<AppendOutcome> {
        let path = self.tape_path(tape);
        self.tape_files
            .entry(tape.to_owned())
            .or_insert_with(|| TapeFile::new(path))
            .append(TapEntry {
                id: 0,
                kind,
                payload,
                timestamp: jiff::Timestamp::now(),
                metadata,
            })
    }

    /// Rename the active tape into a timestamped archive file.
    fn archive(&mut self, tape: &str) -> TapResult<Option<PathBuf>> {
        let mut file = self.take_tape_file(tape);
        file.archive()
    }

    /// Convert a logical tape name into its encoded on-disk path.
    fn tape_path(&self, tape: &str) -> PathBuf {
        let encoded = encode(tape);
        self.tape_root.join(format!(
            "{}__{}{}",
            self.workspace_hash, encoded, TAPE_FILE_SUFFIX
        ))
    }

    /// Remove a cached tape helper from the map, creating one lazily if needed.
    fn take_tape_file(&mut self, tape: &str) -> TapeFile {
        self.tape_files
            .remove(tape)
            .unwrap_or_else(|| TapeFile::new(self.tape_path(tape)))
    }
}

/// Thin async-facing handle for dispatching serialized work onto the blocking
/// tape I/O thread.
#[derive(Clone, Debug)]
struct IoWorker {
    sender: Arc<mpsc::Sender<Job>>,
}

impl IoWorker {
    /// Spawn the dedicated tape I/O thread and wait until it has initialized.
    async fn start(home: &Path, workspace_path: &Path) -> TapResult<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = oneshot::channel::<TapResult<()>>();
        let home = home.to_path_buf();
        let workspace_path = workspace_path.to_path_buf();

        thread::Builder::new()
            .name("rara-tape-io".to_owned())
            .spawn(move || {
                let mut state = match WorkerState::new(&home, &workspace_path) {
                    Ok(state) => {
                        let _ = ready_tx.send(Ok(()));
                        state
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };

                while let Ok(job) = receiver.recv() {
                    job(&mut state);
                }
            })
            .map_err(|source| TapError::State {
                message: source.to_string(),
            })?;

        ready_rx.await.map_err(|_| TapError::State {
            message: "tape I/O worker terminated during startup".to_owned(),
        })??;

        Ok(Self {
            sender: Arc::new(sender),
        })
    }

    /// Send one operation to the worker thread and await its typed response.
    async fn call<T, F>(&self, operation: F) -> TapResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut WorkerState) -> TapResult<T> + Send + 'static,
    {
        let (reply_tx, reply_rx) = oneshot::channel::<TapResult<T>>();
        self.sender
            .send(Box::new(move |state| {
                let _ = reply_tx.send(operation(state));
            }))
            .map_err(|_| TapError::State {
                message: "tape I/O worker is not available".to_owned(),
            })?;

        reply_rx.await.map_err(|_| TapError::State {
            message: "tape I/O worker dropped the response channel".to_owned(),
        })?
    }
}

/// Append-only JSONL tape store compatible with Bub's file tape store
/// semantics.
///
/// The store's async API is a transport layer over a dedicated blocking I/O
/// worker thread. This keeps runtime-facing code non-blocking while ensuring
/// append-only mutations are serialized in one place.
#[derive(Clone, Debug)]
pub struct FileTapeStore {
    worker: IoWorker,
}

impl FileTapeStore {
    /// Create a store rooted at `home/tapes` for one workspace.
    pub async fn new(home: &Path, workspace_path: &Path) -> TapResult<Self> {
        Ok(Self {
            worker: IoWorker::start(home, workspace_path).await?,
        })
    }

    /// List all non-fork tapes known for the current workspace.
    pub async fn list_tapes(&self) -> TapResult<Vec<String>> {
        self.worker.call(WorkerState::list_tapes).await
    }

    /// Create a fork tape derived from `source`.  When `at_entry_id` is
    /// `Some`, only entries from that ID onward are included (partial fork).
    pub async fn fork(&self, source: &str, at_entry_id: Option<u64>) -> TapResult<String> {
        let source = source.to_owned();
        self.worker
            .call(move |state| state.fork(&source, at_entry_id))
            .await
    }

    /// Merge a fork tape back into `target`, appending only fork-local entries.
    pub async fn merge(&self, source: &str, target: &str) -> TapResult<()> {
        let source = source.to_owned();
        let target = target.to_owned();
        self.worker
            .call(move |state| state.merge(&source, &target))
            .await
    }

    /// Discard a fork tape, deleting its file without merging entries back.
    pub async fn discard(&self, fork_tape: &str) -> TapResult<()> {
        let fork_tape = fork_tape.to_owned();
        self.worker
            .call(move |state| state.discard(&fork_tape))
            .await
    }

    /// Delete one active tape file and clear its cache.
    pub async fn reset(&self, tape: &str) -> TapResult<()> {
        let tape = tape.to_owned();
        self.worker.call(move |state| state.reset(&tape)).await
    }

    /// Read all entries from one tape if it exists.
    pub async fn read(&self, tape: &str) -> TapResult<Option<Vec<TapEntry>>> {
        let tape = tape.to_owned();
        self.worker.call(move |state| state.read(&tape)).await
    }

    /// Return the entry ID of the most recent anchor on `tape`, if any.
    ///
    /// Backed by an in-memory secondary index, so this is O(1) once the tape
    /// is loaded — much cheaper than reading every entry only to scan the
    /// result on the caller side.
    pub async fn last_anchor_id(&self, tape: &str) -> TapResult<Option<u64>> {
        let tape = tape.to_owned();
        self.worker
            .call(move |state| state.last_anchor_id(&tape))
            .await
    }

    /// Return the entry ID of the most recent anchor on `tape` whose payload
    /// `name` field matches `anchor_name`.
    ///
    /// Backed by the in-memory anchor-name index, so this is O(1) lookup
    /// rather than O(n) linear scan over all tape entries.
    pub async fn last_anchor_id_by_name(
        &self,
        tape: &str,
        anchor_name: &str,
    ) -> TapResult<Option<u64>> {
        let tape = tape.to_owned();
        let name = anchor_name.to_owned();
        self.worker
            .call(move |state| state.last_anchor_id_by_name(&tape, &name))
            .await
    }

    /// Read every entry on `tape` whose kind matches `kind`, in append order.
    ///
    /// Backed by the kind index so this is O(k) in the number of matches
    /// instead of O(n) over the whole tape.
    pub async fn entries_by_kind(
        &self,
        tape: &str,
        kind: super::TapEntryKind,
    ) -> TapResult<Vec<TapEntry>> {
        let tape = tape.to_owned();
        self.worker
            .call(move |state| state.entries_by_kind(&tape, kind))
            .await
    }

    /// Append one entry to a tape, creating the tape file if needed.
    ///
    /// The returned [`AppendOutcome`] carries the assigned entry id, the
    /// byte offset where this entry's JSONL line starts on disk, and the
    /// total entry count after the append. See `AppendOutcome` for the
    /// motivation (issue #2025 — synchronous session-index update).
    pub async fn append(
        &self,
        tape: &str,
        kind: super::TapEntryKind,
        payload: serde_json::Value,
        metadata: Option<serde_json::Value>,
    ) -> TapResult<AppendOutcome> {
        let tape = tape.to_owned();
        self.worker
            .call(move |state| state.append(&tape, kind, payload, metadata))
            .await
    }

    /// Archive one tape into a timestamped backup file.
    pub async fn archive(&self, tape: &str) -> TapResult<Option<PathBuf>> {
        let tape = tape.to_owned();
        self.worker.call(move |state| state.archive(&tape)).await
    }
}

/// Trim ASCII whitespace around one JSONL record before decoding it.
fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    &bytes[start..end]
}

/// Convert a `rustix` errno into the store's `TapError` shape.
fn rustix_error(source: rustix::io::Errno) -> TapError {
    TapError::Io {
        source: std::io::Error::from_raw_os_error(source.raw_os_error()),
    }
}

/// Generate a collision-resistant suffix for derived tape names.
fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static FORK_COUNTER: AtomicU64 = AtomicU64::new(0);

    let seq = FORK_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    (nanos << 32) | (seq & 0xFFFF_FFFF)
}

/// Hash the canonical workspace path so tape file names stay namespace-scoped.
fn md5_hex(path: &Path) -> String {
    let digest = md5::compute(path.to_string_lossy().as_bytes());
    format!("{digest:x}")
}
