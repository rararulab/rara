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

use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Utc};

use crate::Result;

/// Creates a time-based directory path: `base/YYYY/MM/DD`.
pub fn time_based_dir<P: AsRef<Path>>(base: P, time: DateTime<Utc>) -> PathBuf {
    let base = base.as_ref();
    base.join(format!("{:04}", time.year()))
        .join(format!("{:02}", time.month()))
        .join(format!("{:02}", time.day()))
}

/// Generates a data file name: `YYYYMMDD-NNNN.data`.
#[must_use]
pub fn data_file_name(time: DateTime<Utc>, sequence: u32) -> String {
    format!(
        "{:04}{:02}{:02}-{:04}.data",
        time.year(),
        time.month(),
        time.day(),
        sequence
    )
}

/// Generates an index file name: `YYYYMMDD-NNNN.index`.
#[must_use]
pub fn index_file_name(time: DateTime<Utc>, sequence: u32) -> String {
    format!(
        "{:04}{:02}{:02}-{:04}.index",
        time.year(),
        time.month(),
        time.day(),
        sequence
    )
}

/// Returns full path to a data file: `base/YYYY/MM/DD/YYYYMMDD-NNNN.data`.
pub fn data_file_path<P: AsRef<Path>>(base: P, time: DateTime<Utc>, sequence: u32) -> PathBuf {
    let dir = time_based_dir(base, time);
    dir.join(data_file_name(time, sequence))
}

/// Returns full path to an index file: `base/YYYY/MM/DD/YYYYMMDD-NNNN.index`.
pub fn index_file_path<P: AsRef<Path>>(base: P, time: DateTime<Utc>, sequence: u32) -> PathBuf {
    let dir = time_based_dir(base, time);
    dir.join(index_file_name(time, sequence))
}

/// Recursively scans for all `.data` files under the base directory.
///
/// # Errors
///
/// Returns an error if directory traversal fails or permissions are denied.
pub fn scan_data_files<P: AsRef<Path>>(base: P) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    scan_data_files_recursive(base.as_ref(), &mut files)?;
    files.sort();
    Ok(files)
}

fn scan_data_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            scan_data_files_recursive(&path, files)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("data") {
            files.push(path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn test_time_based_dir() {
        let time = Utc.with_ymd_and_hms(2026, 1, 14, 12, 0, 0).unwrap();
        let dir = time_based_dir("/base", time);
        assert_eq!(dir, PathBuf::from("/base/2026/01/14"));
    }

    #[test]
    fn test_data_file_name() {
        let time = Utc.with_ymd_and_hms(2026, 1, 14, 12, 0, 0).unwrap();
        let name = data_file_name(time, 1);
        assert_eq!(name, "20260114-0001.data");
    }

    #[test]
    fn test_index_file_name() {
        let time = Utc.with_ymd_and_hms(2026, 1, 14, 12, 0, 0).unwrap();
        let name = index_file_name(time, 42);
        assert_eq!(name, "20260114-0042.index");
    }

    #[test]
    fn test_full_paths() {
        let time = Utc.with_ymd_and_hms(2026, 1, 14, 12, 0, 0).unwrap();

        let data_path = data_file_path("/queue", time, 1);
        assert_eq!(
            data_path,
            PathBuf::from("/queue/2026/01/14/20260114-0001.data")
        );

        let idx_path = index_file_path("/queue", time, 1);
        assert_eq!(
            idx_path,
            PathBuf::from("/queue/2026/01/14/20260114-0001.index")
        );
    }

    #[test]
    fn test_scan_data_files() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let base = temp_dir.path();

        let time1 = Utc.with_ymd_and_hms(2026, 1, 14, 0, 0, 0).unwrap();
        let time2 = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();

        let path1 = data_file_path(base, time1, 1);
        let path2 = data_file_path(base, time1, 2);
        let path3 = data_file_path(base, time2, 1);

        std::fs::create_dir_all(path1.parent().unwrap()).unwrap();
        std::fs::create_dir_all(path3.parent().unwrap()).unwrap();

        std::fs::File::create(&path1).unwrap();
        std::fs::File::create(&path2).unwrap();
        std::fs::File::create(&path3).unwrap();

        let files = scan_data_files(base).unwrap();
        assert_eq!(files.len(), 3);
        assert!(files.contains(&path1));
        assert!(files.contains(&path2));
        assert!(files.contains(&path3));
    }
}
