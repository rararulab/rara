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

//! Crash recovery for the persistent queue.
//!
//! Recovery uses the manifest file for O(1) startup:
//! 1. Read manifest to get queue state
//! 2. Incrementally scan only the active file from the recorded position
//! 3. Return [`RecoveryInfo`] with positions for resuming writes
//!
//! For fresh queues (no manifest), returns default state.

use std::path::Path;

use tracing::{debug, info, warn};

use crate::{
    QueueConfig, Result,
    crc::verify_message_crc,
    file::ReadOnlyDataFile,
    manifest::{FileEntry, Manifest},
    manifest_writer::ManifestWriter,
    message::{MESSAGE_CRC_SIZE, MESSAGE_LENGTH_SIZE},
};

/// Information recovered from scanning existing data files.
///
/// Used to initialize the `IOWorker` so it can resume writing from
/// where the previous session left off.
#[derive(Debug, Default)]
pub struct RecoveryInfo {
    /// Sequence number to assign to the next message.
    pub next_sequence:   u64,
    /// File sequence number (from filename, e.g., "20260114-0042" → 42).
    pub file_sequence:   u32,
    /// Byte offset in the last file where the next write should go.
    pub write_position:  u64,
    /// Number of messages in the current (last) file.
    pub message_count:   u64,
    /// Metadata for completed (rolled) data files.
    pub completed_files: Vec<FileEntry>,
}

/// Result of recovery including the manifest writer.
pub struct RecoveryResult {
    pub info:            RecoveryInfo,
    pub manifest_writer: ManifestWriter,
}

/// Perform recovery by reading manifest and scanning active file.
///
/// Returns [`RecoveryResult`] with the state needed to resume operations.
/// If no manifest exists, returns default (fresh start).
pub fn recover(config: &QueueConfig) -> Result<RecoveryResult> {
    info!(path = ?config.base_path, "Starting queue recovery");

    let manifest_writer = ManifestWriter::new(&config.base_path)?;
    let manifest = manifest_writer.read_latest()?;

    let info = match manifest {
        None => {
            info!("No manifest found, starting fresh");
            RecoveryInfo::default()
        }
        Some(manifest) => recover_from_manifest(manifest, config.verify_on_startup)?,
    };

    Ok(RecoveryResult {
        info,
        manifest_writer,
    })
}

fn recover_from_manifest(manifest: Manifest, verify_crc: bool) -> Result<RecoveryInfo> {
    let active_path = &manifest.active_file.path;

    if !active_path.exists() || active_path.as_os_str().is_empty() {
        info!(
            next_sequence = manifest.next_sequence,
            file_sequence = manifest.active_file.file_sequence,
            "Recovery from manifest complete (no active file)"
        );
        return Ok(RecoveryInfo {
            next_sequence:   manifest.next_sequence,
            file_sequence:   manifest.active_file.file_sequence,
            write_position:  0,
            message_count:   0,
            completed_files: manifest.files,
        });
    }

    let (additional_messages, final_position) =
        scan_data_file_from(active_path, manifest.active_file.write_position, verify_crc)?;

    let info = RecoveryInfo {
        next_sequence:   manifest.next_sequence + additional_messages,
        file_sequence:   manifest.active_file.file_sequence,
        write_position:  final_position,
        message_count:   manifest.active_file.message_count + additional_messages,
        completed_files: manifest.files,
    };

    info!(
        next_sequence = info.next_sequence,
        file_sequence = info.file_sequence,
        write_position = info.write_position,
        additional_messages,
        "Recovery from manifest complete"
    );

    Ok(info)
}

/// Scan a data file starting from a given offset.
///
/// Returns `(message_count, final_position)`.
fn scan_data_file_from(path: &Path, start_position: u64, verify_crc: bool) -> Result<(u64, u64)> {
    debug!(path = ?path, start_position, "Scanning data file from offset");

    let file = ReadOnlyDataFile::open(path)?;
    let file_size = file.size();

    let mut position = start_position;
    let mut message_count = 0u64;

    while position + MESSAGE_LENGTH_SIZE as u64 <= file_size {
        let mut length_buf = [0u8; MESSAGE_LENGTH_SIZE];
        file.read_at(position, &mut length_buf)?;
        let length = u32::from_le_bytes(length_buf);

        if length == 0 {
            break;
        }

        let total_size = MESSAGE_LENGTH_SIZE as u64 + u64::from(length) + MESSAGE_CRC_SIZE as u64;

        if position + total_size > file_size {
            warn!(
                position,
                length, file_size, "Truncated message found at end of file"
            );
            break;
        }

        if verify_crc {
            let payload_offset = position + MESSAGE_LENGTH_SIZE as u64;
            let crc_offset = payload_offset + u64::from(length);

            let payload = file.as_slice(payload_offset, u64::from(length))?;

            let mut crc_buf = [0u8; MESSAGE_CRC_SIZE];
            file.read_at(crc_offset, &mut crc_buf)?;
            let stored_crc = u32::from_le_bytes(crc_buf);

            if !verify_message_crc(length, payload, stored_crc) {
                warn!(
                    position,
                    sequence = message_count,
                    "CRC verification failed, stopping recovery at this point"
                );
                break;
            }
        }

        position += total_size;
        message_count += 1;
    }

    debug!(
        path = ?path,
        messages = message_count,
        position,
        "File scan complete"
    );

    Ok((message_count, position))
}

#[cfg(test)]
fn extract_file_sequence(path: &Path) -> Result<u32> {
    use snafu::OptionExt;

    use crate::error::InvalidPathSnafu;

    let filename = path
        .file_stem()
        .and_then(|s| s.to_str())
        .context(InvalidPathSnafu {
            path: path.to_path_buf(),
        })?;

    let parts: Vec<&str> = filename.split('-').collect();

    if parts.len() != 2 {
        return InvalidPathSnafu {
            path: path.to_path_buf(),
        }
        .fail();
    }

    parts[1].parse().ok().context(InvalidPathSnafu {
        path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;
    use tempfile::TempDir;
    use test_case::test_case;

    use super::*;
    use crate::{
        FlushMode, RollStrategy,
        crc::calculate_message_crc,
        file::DataFile,
        manifest::{ActiveFileState, MANIFEST_VERSION, Manifest},
        path::data_file_path,
    };

    fn test_config(base_path: PathBuf) -> QueueConfig {
        QueueConfig {
            base_path,
            file_size: 1024 * 1024,
            roll_strategy: RollStrategy::BySize(1024 * 1024),
            flush_mode: FlushMode::Sync,
            index_interval: 10,
            verify_on_startup: true,
        }
    }

    fn write_test_message(file: &DataFile, offset: u64, data: &[u8]) -> u64 {
        let length = data.len() as u32;
        let crc = calculate_message_crc(length, data);

        file.write_at(offset, &length.to_le_bytes()).unwrap();
        file.write_at(offset + 4, data).unwrap();
        file.write_at(offset + 4 + data.len() as u64, &crc.to_le_bytes())
            .unwrap();

        4 + data.len() as u64 + 4
    }

    struct TestFixture {
        _temp_dir: TempDir,
        config:    QueueConfig,
        data_path: PathBuf,
    }

    impl TestFixture {
        fn new() -> Self {
            let temp_dir = TempDir::new().unwrap();
            let config = test_config(temp_dir.path().to_path_buf());
            let now = Utc::now();
            let data_path = data_file_path(&config.base_path, now, 1);
            Self {
                _temp_dir: temp_dir,
                config,
                data_path,
            }
        }

        fn write_messages(&self, count: usize) -> (DataFile, u64) {
            let file = DataFile::create(&self.data_path, 4096).unwrap();
            let mut offset = 0u64;
            for i in 0..count {
                let msg = format!("msg-{i}");
                offset += write_test_message(&file, offset, msg.as_bytes());
            }
            file.flush(&FlushMode::Sync).unwrap();
            (file, offset)
        }

        fn write_manifest(&self, next_sequence: u64, write_position: u64, message_count: u64) {
            let manifest = Manifest {
                version: MANIFEST_VERSION,
                next_sequence,
                active_file: ActiveFileState {
                    file_sequence: 1,
                    write_position,
                    message_count,
                    path: self.data_path.clone(),
                },
                files: vec![],
            };
            let mut writer = ManifestWriter::new(&self.config.base_path).unwrap();
            writer.write(&manifest).unwrap();
        }
    }

    #[test]
    fn test_recovery_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(temp_dir.path().to_path_buf());

        let result = recover(&config).unwrap();

        assert_eq!(result.info.next_sequence, 0);
        assert_eq!(result.info.file_sequence, 0);
        assert_eq!(result.info.write_position, 0);
    }

    #[test]
    fn test_recovery_from_manifest() {
        let fixture = TestFixture::new();
        let (_file, offset) = fixture.write_messages(10);
        fixture.write_manifest(10, offset, 10);

        let result = recover(&fixture.config).unwrap();

        assert_eq!(result.info.next_sequence, 10);
        assert_eq!(result.info.file_sequence, 1);
        assert_eq!(result.info.write_position, offset);
        assert_eq!(result.info.message_count, 10);
    }

    #[test]
    fn test_recovery_with_additional_messages() {
        let fixture = TestFixture::new();
        let file = DataFile::create(&fixture.data_path, 4096).unwrap();

        let mut offset = 0u64;
        for i in 0..5 {
            offset += write_test_message(&file, offset, format!("msg-{i}").as_bytes());
        }
        let manifest_offset = offset;

        for i in 5..10 {
            offset += write_test_message(&file, offset, format!("msg-{i}").as_bytes());
        }
        file.flush(&FlushMode::Sync).unwrap();

        fixture.write_manifest(5, manifest_offset, 5);

        let result = recover(&fixture.config).unwrap();

        assert_eq!(result.info.next_sequence, 10);
        assert_eq!(result.info.file_sequence, 1);
        assert_eq!(result.info.write_position, offset);
        assert_eq!(result.info.message_count, 10);
    }

    #[test]
    fn test_recovery_stops_at_corrupted_message() {
        let fixture = TestFixture::new();
        let file = DataFile::create(&fixture.data_path, 4096).unwrap();

        let mut offset = 0u64;
        for i in 0..5 {
            offset += write_test_message(&file, offset, format!("msg-{i}").as_bytes());
        }
        let manifest_offset = offset;

        for i in 5..8 {
            offset += write_test_message(&file, offset, format!("msg-{i}").as_bytes());
        }
        let valid_offset = offset;

        let bad_data = b"corrupted";
        let bad_crc = 0xDEAD_BEEF_u64;
        file.write_at(offset, &(bad_data.len() as u32).to_le_bytes())
            .unwrap();
        file.write_at(offset + 4, bad_data).unwrap();
        file.write_at(offset + 4 + bad_data.len() as u64, &bad_crc.to_le_bytes())
            .unwrap();
        file.flush(&FlushMode::Sync).unwrap();

        fixture.write_manifest(5, manifest_offset, 5);

        let result = recover(&fixture.config).unwrap();

        assert_eq!(result.info.next_sequence, 8);
        assert_eq!(result.info.write_position, valid_offset);
        assert_eq!(result.info.message_count, 8);
    }

    #[test_case("/queue/2026/01/14/20260114-0042.data", 42 ; "sequence 42")]
    #[test_case("/queue/2026/01/14/20260114-0001.data", 1 ; "sequence 1")]
    #[test_case("/queue/2026/12/31/20261231-9999.data", 9999 ; "sequence 9999")]
    fn test_extract_file_sequence(path: &str, expected: u32) {
        assert_eq!(extract_file_sequence(Path::new(path)).unwrap(), expected);
    }
}
