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

#![allow(clippy::missing_errors_doc)]

use std::path::{Path, PathBuf};

use mmap_io::MemoryMappedFile;
use snafu::ResultExt;

use crate::{FlushMode, Result, error::MmapFailedSnafu};

pub struct DataFile {
    mmap: MemoryMappedFile,
    path: PathBuf,
    size: u64,
}

impl DataFile {
    pub fn create<P: AsRef<Path>>(path: P, size: u64) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mmap = MemoryMappedFile::create_rw(&path, size).context(MmapFailedSnafu)?;

        Ok(Self { mmap, path, size })
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        let mmap = MemoryMappedFile::open_rw(&path).context(MmapFailedSnafu)?;

        let size = mmap.len();

        Ok(Self { mmap, path, size })
    }

    #[inline]
    pub fn write_at(&self, offset: u64, data: &[u8]) -> Result<()> {
        self.mmap
            .update_region(offset, data)
            .context(MmapFailedSnafu)
    }

    #[inline]
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.mmap.read_into(offset, buf).context(MmapFailedSnafu)
    }

    #[must_use]
    pub const fn size(&self) -> u64 { self.size }

    #[must_use]
    pub fn path(&self) -> &Path { &self.path }

    pub fn flush(&self, mode: &FlushMode) -> Result<()> {
        match mode {
            FlushMode::Async | FlushMode::Sync | FlushMode::Batch { .. } => {
                self.mmap.flush().context(MmapFailedSnafu)?;
            }
        }
        Ok(())
    }

    pub fn flush_range(&self, offset: u64, len: u64) -> Result<()> {
        self.mmap.flush_range(offset, len).context(MmapFailedSnafu)
    }
}

pub struct ReadOnlyDataFile {
    mmap: MemoryMappedFile,
    size: u64,
}

impl ReadOnlyDataFile {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mmap = MemoryMappedFile::open_ro(path.as_ref()).context(MmapFailedSnafu)?;

        let size = mmap.len();

        Ok(Self { mmap, size })
    }

    #[inline]
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.mmap.read_into(offset, buf).context(MmapFailedSnafu)
    }

    #[inline]
    pub fn as_slice(&self, offset: u64, len: u64) -> Result<&[u8]> {
        self.mmap.as_slice(offset, len).context(MmapFailedSnafu)
    }

    #[must_use]
    pub const fn size(&self) -> u64 { self.size }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_create_data_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.data");
        let size = 4096;

        let file = DataFile::create(&path, size).unwrap();
        assert_eq!(file.size(), size);
        assert!(path.exists());
    }

    #[test]
    fn test_write_and_read_data_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.data");
        let size = 4096;

        {
            let file = DataFile::create(&path, size).unwrap();
            let data = b"Hello, World!";
            file.write_at(0, data).unwrap();
            file.flush(&FlushMode::Sync).unwrap();
        }

        {
            let file = ReadOnlyDataFile::open(&path).unwrap();
            let data = file.as_slice(0, 13).unwrap();
            assert_eq!(data, b"Hello, World!");
        }
    }

    #[test]
    fn test_read_into_buffer() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.data");
        let size = 4096;

        {
            let file = DataFile::create(&path, size).unwrap();
            file.write_at(100, b"Test data at offset").unwrap();
            file.flush(&FlushMode::Sync).unwrap();
        }

        {
            let file = ReadOnlyDataFile::open(&path).unwrap();
            let mut buf = [0u8; 19];
            file.read_at(100, &mut buf).unwrap();
            assert_eq!(&buf, b"Test data at offset");
        }
    }

    #[test]
    fn test_open_existing_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.data");

        {
            DataFile::create(&path, 1024).unwrap();
        }

        let file = DataFile::open(&path).unwrap();
        assert_eq!(file.size(), 1024);
    }
}
