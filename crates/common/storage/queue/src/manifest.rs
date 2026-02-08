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

#![allow(clippy::cast_possible_truncation)]

//! Manifest data structures for O(1) queue recovery.
//!
//! The manifest records queue metadata to avoid scanning all data files on
//! startup. It tracks the global sequence number, active file state, and all
//! historical files.
//!
//! ## Binary Format
//!
//! ```text
//! ┌────────────────────────────────────────────────────────┐
//! │ Header (32 bytes)                                       │
//! ├─────────────────┬──────────────────────────────────────┤
//! │ magic: [u8; 4]  │ "QMFT" (0x514D4654)                  │
//! │ version: u32    │ Format version, currently 1          │
//! │ next_seq: u64   │ Next global sequence number          │
//! │ file_count: u32 │ Number of FileEntry records          │
//! │ checksum: u32   │ CRC32 of content after header        │
//! │ reserved: [u8;8]│ Reserved for future use              │
//! ├─────────────────┴──────────────────────────────────────┤
//! │ Active File State (variable)                           │
//! │ FileEntry[] (variable)                                 │
//! └────────────────────────────────────────────────────────┘
//! ```

use std::{
    io::{Read, Write},
    path::PathBuf,
};

use crc32fast::Hasher;
use snafu::ensure;

use crate::{
    Result,
    error::{ManifestCorruptedSnafu, UnsupportedManifestVersionSnafu},
};

/// Magic bytes identifying a manifest file: "QMFT"
pub const MANIFEST_MAGIC: [u8; 4] = [0x51, 0x4D, 0x46, 0x54];

/// Current manifest format version.
pub const MANIFEST_VERSION: u32 = 1;

/// Size of the manifest header in bytes.
pub const MANIFEST_HEADER_SIZE: usize = 32;

/// Queue manifest containing all metadata for O(1) recovery.
#[derive(Debug, Clone)]
pub struct Manifest {
    /// Format version for forward compatibility.
    pub version:       u32,
    /// Next sequence number to assign to incoming messages.
    pub next_sequence: u64,
    /// State of the currently active (writable) file.
    pub active_file:   ActiveFileState,
    /// Metadata for all completed (read-only) data files.
    pub files:         Vec<FileEntry>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version:       MANIFEST_VERSION,
            next_sequence: 0,
            active_file:   ActiveFileState::default(),
            files:         Vec::new(),
        }
    }
}

/// State of the active file being written to.
#[derive(Debug, Clone, Default)]
pub struct ActiveFileState {
    /// File sequence number (from filename, e.g., YYYYMMDD-NNNN).
    pub file_sequence:  u32,
    /// Byte offset where the next write will occur.
    pub write_position: u64,
    /// Number of messages written to this file.
    pub message_count:  u64,
    /// Path to the active data file.
    pub path:           PathBuf,
}

/// Metadata for a completed data file.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Path to the data file.
    pub path:           PathBuf,
    /// Sequence number of the first message in this file.
    pub start_sequence: u64,
    /// Sequence number of the last message in this file.
    pub end_sequence:   u64,
    /// File size in bytes.
    pub size:           u64,
}

impl Manifest {
    /// Serialize the manifest to bytes.
    ///
    /// Format:
    /// - Header (32 bytes): magic, version, `next_seq`, `file_count`, checksum,
    ///   reserved
    /// - Active file state (variable)
    /// - File entries (variable)
    pub fn serialize(&self) -> Vec<u8> {
        let mut content = Vec::new();
        self.active_file.write_to(&mut content);
        for entry in &self.files {
            entry.write_to(&mut content);
        }

        let checksum = {
            let mut hasher = Hasher::new();
            hasher.update(&content);
            hasher.finalize()
        };

        let mut header = [0u8; MANIFEST_HEADER_SIZE];
        header[0..4].copy_from_slice(&MANIFEST_MAGIC);
        header[4..8].copy_from_slice(&self.version.to_le_bytes());
        header[8..16].copy_from_slice(&self.next_sequence.to_le_bytes());
        header[16..20].copy_from_slice(&(self.files.len() as u32).to_le_bytes());
        header[20..24].copy_from_slice(&checksum.to_le_bytes());

        let mut result = Vec::with_capacity(MANIFEST_HEADER_SIZE + content.len());
        result.extend_from_slice(&header);
        result.extend(content);
        result
    }

    /// Deserialize a manifest from bytes.
    ///
    /// Validates magic, version, and checksum. Returns error if any check
    /// fails.
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        ensure!(
            data.len() >= MANIFEST_HEADER_SIZE,
            ManifestCorruptedSnafu {
                reason: format!(
                    "data too short: {} bytes, expected at least {}",
                    data.len(),
                    MANIFEST_HEADER_SIZE
                ),
            }
        );

        let magic = &data[0..4];
        ensure!(
            magic == MANIFEST_MAGIC,
            ManifestCorruptedSnafu {
                reason: format!("invalid magic: {magic:?}"),
            }
        );

        let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
        ensure!(
            version == MANIFEST_VERSION,
            UnsupportedManifestVersionSnafu { version }
        );

        let next_sequence = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let file_count = u32::from_le_bytes(data[16..20].try_into().unwrap());
        let stored_checksum = u32::from_le_bytes(data[20..24].try_into().unwrap());

        let content = &data[MANIFEST_HEADER_SIZE..];
        let computed_checksum = {
            let mut hasher = Hasher::new();
            hasher.update(content);
            hasher.finalize()
        };

        ensure!(
            stored_checksum == computed_checksum,
            ManifestCorruptedSnafu {
                reason: format!(
                    "checksum mismatch: stored={stored_checksum:#x}, \
                     computed={computed_checksum:#x}"
                ),
            }
        );

        let mut cursor = std::io::Cursor::new(content);
        let active_file = ActiveFileState::read_from(&mut cursor)?;

        let mut files = Vec::with_capacity(file_count as usize);
        for _ in 0..file_count {
            files.push(FileEntry::read_from(&mut cursor)?);
        }

        Ok(Self {
            version,
            next_sequence,
            active_file,
            files,
        })
    }
}

impl ActiveFileState {
    /// Write active file state to a writer.
    fn write_to<W: Write>(&self, writer: &mut W) {
        let path_bytes = self.path.to_string_lossy().as_bytes().to_vec();
        let path_len = path_bytes.len() as u16;

        writer.write_all(&self.file_sequence.to_le_bytes()).unwrap();
        writer
            .write_all(&self.write_position.to_le_bytes())
            .unwrap();
        writer.write_all(&self.message_count.to_le_bytes()).unwrap();
        writer.write_all(&path_len.to_le_bytes()).unwrap();
        writer.write_all(&path_bytes).unwrap();
    }

    /// Read active file state from a reader.
    fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let mut buf4 = [0u8; 4];
        let mut buf8 = [0u8; 8];
        let mut buf2 = [0u8; 2];

        reader.read_exact(&mut buf4)?;
        let file_sequence = u32::from_le_bytes(buf4);

        reader.read_exact(&mut buf8)?;
        let write_position = u64::from_le_bytes(buf8);

        reader.read_exact(&mut buf8)?;
        let message_count = u64::from_le_bytes(buf8);

        reader.read_exact(&mut buf2)?;
        let path_len = u16::from_le_bytes(buf2) as usize;

        let mut path_bytes = vec![0u8; path_len];
        reader.read_exact(&mut path_bytes)?;
        let path = PathBuf::from(String::from_utf8_lossy(&path_bytes).into_owned());

        Ok(Self {
            file_sequence,
            write_position,
            message_count,
            path,
        })
    }
}

impl FileEntry {
    /// Write file entry to a writer.
    fn write_to<W: Write>(&self, writer: &mut W) {
        let path_bytes = self.path.to_string_lossy().as_bytes().to_vec();
        let path_len = path_bytes.len() as u16;

        writer
            .write_all(&self.start_sequence.to_le_bytes())
            .unwrap();
        writer.write_all(&self.end_sequence.to_le_bytes()).unwrap();
        writer.write_all(&self.size.to_le_bytes()).unwrap();
        writer.write_all(&path_len.to_le_bytes()).unwrap();
        writer.write_all(&path_bytes).unwrap();
    }

    /// Read file entry from a reader.
    fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let mut buf8 = [0u8; 8];
        let mut buf2 = [0u8; 2];

        reader.read_exact(&mut buf8)?;
        let start_sequence = u64::from_le_bytes(buf8);

        reader.read_exact(&mut buf8)?;
        let end_sequence = u64::from_le_bytes(buf8);

        reader.read_exact(&mut buf8)?;
        let size = u64::from_le_bytes(buf8);

        reader.read_exact(&mut buf2)?;
        let path_len = u16::from_le_bytes(buf2) as usize;

        let mut path_bytes = vec![0u8; path_len];
        reader.read_exact(&mut path_bytes)?;
        let path = PathBuf::from(String::from_utf8_lossy(&path_bytes).into_owned());

        Ok(Self {
            path,
            start_sequence,
            end_sequence,
            size,
        })
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    #[test]
    fn test_manifest_default() {
        let manifest = Manifest::default();
        assert_eq!(manifest.version, MANIFEST_VERSION);
        assert_eq!(manifest.next_sequence, 0);
        assert!(manifest.files.is_empty());
    }

    #[test]
    fn test_manifest_roundtrip_empty() {
        let manifest = Manifest::default();
        let bytes = manifest.serialize();
        let recovered = Manifest::deserialize(&bytes).unwrap();

        assert_eq!(recovered.version, manifest.version);
        assert_eq!(recovered.next_sequence, manifest.next_sequence);
        assert!(recovered.files.is_empty());
    }

    #[test]
    fn test_manifest_roundtrip_with_data() {
        let manifest = Manifest {
            version:       MANIFEST_VERSION,
            next_sequence: 12345,
            active_file:   ActiveFileState {
                file_sequence:  42,
                write_position: 1024,
                message_count:  100,
                path:           PathBuf::from("/data/2026/01/16/20260116-0042.data"),
            },
            files:         vec![
                FileEntry {
                    path:           PathBuf::from("/data/2026/01/15/20260115-0001.data"),
                    start_sequence: 0,
                    end_sequence:   999,
                    size:           1024 * 1024,
                },
                FileEntry {
                    path:           PathBuf::from("/data/2026/01/15/20260115-0002.data"),
                    start_sequence: 1000,
                    end_sequence:   1999,
                    size:           1024 * 1024,
                },
            ],
        };

        let bytes = manifest.serialize();
        let recovered = Manifest::deserialize(&bytes).unwrap();

        assert_eq!(recovered.version, MANIFEST_VERSION);
        assert_eq!(recovered.next_sequence, 12345);
        assert_eq!(recovered.active_file.file_sequence, 42);
        assert_eq!(recovered.active_file.write_position, 1024);
        assert_eq!(recovered.active_file.message_count, 100);
        assert_eq!(
            recovered.active_file.path,
            PathBuf::from("/data/2026/01/16/20260116-0042.data")
        );
        assert_eq!(recovered.files.len(), 2);
        assert_eq!(recovered.files[0].start_sequence, 0);
        assert_eq!(recovered.files[0].end_sequence, 999);
        assert_eq!(recovered.files[1].start_sequence, 1000);
        assert_eq!(recovered.files[1].end_sequence, 1999);
    }

    fn corrupt_magic(bytes: &mut [u8]) { bytes[0] = 0xFF; }

    fn corrupt_checksum(bytes: &mut [u8]) { bytes[20] ^= 0xFF; }

    #[test_case(corrupt_magic ; "invalid magic")]
    #[test_case(corrupt_checksum ; "invalid checksum")]
    fn test_manifest_deserialize_corrupted(corrupt_fn: fn(&mut [u8])) {
        let mut bytes = Manifest::default().serialize();
        corrupt_fn(&mut bytes);
        assert!(Manifest::deserialize(&bytes).is_err());
    }

    #[test]
    fn test_manifest_deserialize_too_short() {
        let bytes = vec![0u8; 10];
        assert!(Manifest::deserialize(&bytes).is_err());
    }
}
