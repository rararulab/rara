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

//! Local filesystem operations for Typst projects.
//!
//! All file reads/writes go directly to the user's local disk.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::error::TypstError;

/// A file tree entry representing a file or directory on disk.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct FileEntry {
    /// Relative path within the project root.
    pub path:     String,
    /// Whether this entry is a directory.
    pub is_dir:   bool,
    /// Child entries (present only for directories).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileEntry>>,
}

/// Validate that a relative path does not escape the project root.
///
/// Returns the canonicalized absolute path on success.
pub fn validate_path(root: &Path, relative: &str) -> Result<PathBuf, TypstError> {
    if relative.is_empty() {
        return Err(TypstError::InvalidRequest {
            message: "file path must not be empty".to_owned(),
        });
    }

    // Reject obvious traversal attempts before touching the filesystem.
    if relative.contains("..") || relative.starts_with('/') {
        return Err(TypstError::PathTraversal {
            path: relative.to_owned(),
        });
    }

    let full = root.join(relative);

    // For files that do not yet exist we need to check the parent.
    let check_path = if full.exists() {
        full.canonicalize()
            .map_err(|e| TypstError::FileIo { source: e })?
    } else {
        // Ensure parent directory exists.
        let parent = full.parent().ok_or_else(|| TypstError::PathTraversal {
            path: relative.to_owned(),
        })?;
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| TypstError::FileIo { source: e })?;
        }
        let canonical_parent = parent
            .canonicalize()
            .map_err(|e| TypstError::FileIo { source: e })?;
        canonical_parent.join(full.file_name().ok_or_else(|| TypstError::PathTraversal {
            path: relative.to_owned(),
        })?)
    };

    let canonical_root = root
        .canonicalize()
        .map_err(|e| TypstError::FileIo { source: e })?;

    if !check_path.starts_with(&canonical_root) {
        return Err(TypstError::PathTraversal {
            path: relative.to_owned(),
        });
    }

    Ok(check_path)
}

/// Recursively scan a directory and return a tree of [`FileEntry`] items.
///
/// Hidden directories (e.g. `.git`) are skipped.
pub fn scan_directory(root: &Path) -> Result<Vec<FileEntry>, TypstError> {
    if !root.exists() {
        return Err(TypstError::DirectoryNotFound {
            path: root.display().to_string(),
        });
    }
    if !root.is_dir() {
        return Err(TypstError::NotADirectory {
            path: root.display().to_string(),
        });
    }

    scan_recursive(root, root)
}

fn scan_recursive(root: &Path, dir: &Path) -> Result<Vec<FileEntry>, TypstError> {
    let mut entries = Vec::new();

    let read_dir = std::fs::read_dir(dir).map_err(|e| TypstError::FileIo { source: e })?;

    let mut dir_entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
    dir_entries.sort_by_key(|e| e.file_name());

    for entry in dir_entries {
        let path = entry.path();

        // Skip hidden entries.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }

        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        if path.is_dir() {
            let children = scan_recursive(root, &path)?;
            entries.push(FileEntry {
                path:     relative,
                is_dir:   true,
                children: Some(children),
            });
        } else if path.is_file() {
            entries.push(FileEntry {
                path:     relative,
                is_dir:   false,
                children: None,
            });
        }
    }

    Ok(entries)
}

/// Read a file's content from disk.
pub fn read_file(root: &Path, relative: &str) -> Result<String, TypstError> {
    let path = validate_path(root, relative)?;
    if !path.exists() {
        return Err(TypstError::FileNotFound {
            path: relative.to_owned(),
        });
    }
    std::fs::read_to_string(&path).map_err(|e| TypstError::FileIo { source: e })
}

/// Write content to a file on disk. Parent directories are created as needed.
pub fn write_file(root: &Path, relative: &str, content: &str) -> Result<(), TypstError> {
    let path = validate_path(root, relative)?;
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| TypstError::FileIo { source: e })?;
        }
    }
    std::fs::write(&path, content).map_err(|e| TypstError::FileIo { source: e })
}

/// Recursively collect all `.typ` file contents under the project root.
///
/// Returns a map of `{relative_path: content}`.
pub fn collect_typ_files(root: &Path) -> Result<HashMap<String, String>, TypstError> {
    let mut map = HashMap::new();
    collect_recursive(root, root, &mut map)?;
    Ok(map)
}

fn collect_recursive(
    root: &Path,
    dir: &Path,
    map: &mut HashMap<String, String>,
) -> Result<(), TypstError> {
    let read_dir = std::fs::read_dir(dir).map_err(|e| TypstError::FileIo { source: e })?;

    for entry in read_dir {
        let entry = entry.map_err(|e| TypstError::FileIo { source: e })?;
        let path = entry.path();

        // Skip hidden directories.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }

        if path.is_dir() {
            collect_recursive(root, &path, map)?;
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "typ" {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                let content =
                    std::fs::read_to_string(&path).map_err(|e| TypstError::FileIo { source: e })?;
                map.insert(relative, content);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn test_scan_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("main.typ"), "hello").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/style.typ"), "style").unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), "ignored").unwrap();

        let entries = scan_directory(root).unwrap();

        // Should have main.typ and sub/ (no .git/)
        assert_eq!(entries.len(), 2);

        let main_entry = entries.iter().find(|e| e.path == "main.typ").unwrap();
        assert!(!main_entry.is_dir);

        let sub_entry = entries.iter().find(|e| e.path == "sub").unwrap();
        assert!(sub_entry.is_dir);
        let children = sub_entry.children.as_ref().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].path, "sub/style.typ");
    }

    #[test]
    fn test_read_write_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "main.typ", "hello world").unwrap();
        let content = read_file(root, "main.typ").unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_write_creates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "a/b/c.typ", "nested").unwrap();
        let content = read_file(root, "a/b/c.typ").unwrap();
        assert_eq!(content, "nested");
    }

    #[test]
    fn test_path_traversal_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let result = validate_path(root, "../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn test_collect_typ_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("main.typ"), "main content").unwrap();
        fs::create_dir(root.join("lib")).unwrap();
        fs::write(root.join("lib/utils.typ"), "utils content").unwrap();
        fs::write(root.join("readme.md"), "not collected").unwrap();

        let map = collect_typ_files(root).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("main.typ").unwrap(), "main content");
        assert_eq!(map.get("lib/utils.typ").unwrap(), "utils content");
    }
}
