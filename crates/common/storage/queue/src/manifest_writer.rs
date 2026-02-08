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

//! Atomic manifest writer using dual-file strategy.
//!
//! Maintains two manifest files and alternates between them to ensure
//! at least one valid manifest exists at any crash point.

use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use tracing::debug;

use crate::{Result, error::ManifestCorruptedSnafu, manifest::Manifest};

const MANIFEST_1: &str = "manifest.1";
const MANIFEST_2: &str = "manifest.2";
const MANIFEST_CURRENT: &str = "manifest.current";

pub(crate) struct ManifestWriter {
    base_path:    PathBuf,
    current_slot: u8,
}

impl ManifestWriter {
    pub fn new<P: AsRef<Path>>(base_path: P) -> Result<Self> {
        let base_path = base_path.as_ref().to_path_buf();
        let current_path = base_path.join(MANIFEST_CURRENT);

        let current_slot = if current_path.exists() {
            let mut file = File::open(&current_path)?;
            let mut buf = [0u8; 1];
            file.read_exact(&mut buf)?;
            buf[0]
        } else {
            0
        };

        Ok(Self {
            base_path,
            current_slot,
        })
    }

    pub fn write(&mut self, manifest: &Manifest) -> Result<()> {
        let next_slot = if self.current_slot == 1 { 2 } else { 1 };
        let manifest_path = self.slot_path(next_slot);
        let current_path = self.base_path.join(MANIFEST_CURRENT);

        let data = manifest.serialize();

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&manifest_path)?;
        file.write_all(&data)?;
        file.sync_all()?;

        let mut current_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&current_path)?;
        current_file.write_all(&[next_slot])?;
        current_file.sync_all()?;

        self.current_slot = next_slot;

        debug!(slot = next_slot, path = ?manifest_path, "Manifest written");
        Ok(())
    }

    pub fn read_latest(&self) -> Result<Option<Manifest>> {
        let current_path = self.base_path.join(MANIFEST_CURRENT);

        if !current_path.exists() {
            return Ok(None);
        }

        let mut file = File::open(&current_path)?;
        let mut buf = [0u8; 1];
        file.read_exact(&mut buf)?;
        let slot = buf[0];

        if slot != 1 && slot != 2 {
            return ManifestCorruptedSnafu {
                reason: format!("invalid slot number in manifest.current: {slot}"),
            }
            .fail();
        }

        let manifest_path = self.slot_path(slot);
        if !manifest_path.exists() {
            return ManifestCorruptedSnafu {
                reason: format!("manifest.{slot} does not exist"),
            }
            .fail();
        }

        let mut file = File::open(&manifest_path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        let manifest = Manifest::deserialize(&data)?;
        Ok(Some(manifest))
    }

    fn slot_path(&self, slot: u8) -> PathBuf {
        match slot {
            1 => self.base_path.join(MANIFEST_1),
            2 => self.base_path.join(MANIFEST_2),
            _ => unreachable!("invalid slot: {}", slot),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use test_case::test_case;

    use super::*;
    use crate::manifest::{ActiveFileState, FileEntry, MANIFEST_VERSION};

    fn create_manifest(next_sequence: u64, file_sequence: u32) -> Manifest {
        Manifest {
            version: MANIFEST_VERSION,
            next_sequence,
            active_file: ActiveFileState {
                file_sequence,
                write_position: next_sequence * 10,
                message_count: next_sequence,
                path: PathBuf::from(format!("test-{file_sequence}.data")),
            },
            files: vec![],
        }
    }

    #[test]
    fn test_fresh_start_returns_none() {
        let temp_dir = TempDir::new().unwrap();
        let writer = ManifestWriter::new(temp_dir.path()).unwrap();
        assert!(writer.read_latest().unwrap().is_none());
    }

    #[test]
    fn test_write_and_read() {
        let temp_dir = TempDir::new().unwrap();
        let mut writer = ManifestWriter::new(temp_dir.path()).unwrap();

        let manifest = create_manifest(100, 1);
        writer.write(&manifest).unwrap();

        let recovered = writer.read_latest().unwrap().unwrap();
        assert_eq!(recovered.next_sequence, 100);
        assert_eq!(recovered.active_file.file_sequence, 1);
    }

    #[test_case(1, 1 ; "first write goes to slot 1")]
    #[test_case(2, 2 ; "second write goes to slot 2")]
    #[test_case(3, 1 ; "third write returns to slot 1")]
    fn test_slot_alternation(write_count: usize, expected_slot: u8) {
        let temp_dir = TempDir::new().unwrap();
        let mut writer = ManifestWriter::new(temp_dir.path()).unwrap();

        for i in 0..write_count {
            let manifest = create_manifest((i + 1) as u64 * 100, (i + 1) as u32);
            writer.write(&manifest).unwrap();
        }

        assert_eq!(writer.current_slot, expected_slot);
    }

    #[test]
    fn test_persistence_across_instances() {
        let temp_dir = TempDir::new().unwrap();

        {
            let mut writer = ManifestWriter::new(temp_dir.path()).unwrap();
            let manifest = Manifest {
                version:       MANIFEST_VERSION,
                next_sequence: 500,
                active_file:   ActiveFileState {
                    file_sequence:  5,
                    write_position: 1024,
                    message_count:  50,
                    path:           PathBuf::from("/data/test.data"),
                },
                files:         vec![FileEntry {
                    path:           PathBuf::from("/data/old.data"),
                    start_sequence: 0,
                    end_sequence:   99,
                    size:           4096,
                }],
            };
            writer.write(&manifest).unwrap();
        }

        let writer = ManifestWriter::new(temp_dir.path()).unwrap();
        let recovered = writer.read_latest().unwrap().unwrap();
        assert_eq!(recovered.next_sequence, 500);
        assert_eq!(recovered.active_file.file_sequence, 5);
        assert_eq!(recovered.files.len(), 1);
        assert_eq!(recovered.files[0].end_sequence, 99);
    }
}
