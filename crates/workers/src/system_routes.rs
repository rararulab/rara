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

//! System-level HTTP routes that don't belong to any domain crate.

use std::path::PathBuf;

use axum::{Json, Router, extract::Query, http::StatusCode, routing::get};
use serde::{Deserialize, Serialize};

/// Build `/api/v1/system/...` routes.
pub fn routes() -> Router {
    Router::new().nest(
        "/api/v1/system",
        Router::new().route("/browse", get(browse_directory)),
    )
}

#[derive(Debug, Deserialize)]
struct BrowseParams {
    /// Directory path to browse.  Falls back to `$HOME` when absent.
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct BrowseResult {
    current_path: String,
    parent_path: Option<String>,
    entries: Vec<DirEntry>,
}

#[derive(Debug, Serialize)]
struct DirEntry {
    name: String,
    path: String,
    has_typ_files: bool,
}

/// `GET /api/v1/system/browse?path=...`
///
/// Returns the list of sub-directories at the given path so the frontend
/// folder-picker can navigate the local filesystem.
async fn browse_directory(
    Query(params): Query<BrowseParams>,
) -> Result<Json<BrowseResult>, (StatusCode, String)> {
    let dir = match params.path {
        Some(ref p) if !p.is_empty() => PathBuf::from(p),
        _ => home_dir().ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not determine home directory".to_string(),
            )
        })?,
    };

    // Canonicalize so the UI always sees clean absolute paths.
    let dir = dir.canonicalize().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => (
            StatusCode::NOT_FOUND,
            format!("path not found: {}", dir.display()),
        ),
        std::io::ErrorKind::PermissionDenied => (
            StatusCode::FORBIDDEN,
            format!("permission denied: {}", dir.display()),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot resolve path: {e}"),
        ),
    })?;

    if !dir.is_dir() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("not a directory: {}", dir.display()),
        ));
    }

    let read_dir = std::fs::read_dir(&dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => (
            StatusCode::FORBIDDEN,
            format!("permission denied: {}", dir.display()),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read directory: {e}"),
        ),
    })?;

    let mut entries: Vec<DirEntry> = Vec::new();

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let file_name = entry.file_name();
        let name = file_name.to_string_lossy().to_string();

        // Skip hidden directories.
        if name.starts_with('.') {
            continue;
        }

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if !ft.is_dir() {
            continue;
        }

        let child_path = entry.path();
        let has_typ_files = has_typ_files_in(&child_path);

        entries.push(DirEntry {
            name,
            path: child_path.to_string_lossy().to_string(),
            has_typ_files,
        });
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    let parent_path = dir.parent().map(|p| p.to_string_lossy().to_string());

    Ok(Json(BrowseResult {
        current_path: dir.to_string_lossy().to_string(),
        parent_path,
        entries,
    }))
}

/// Quick (non-recursive) check whether `dir` contains at least one `.typ` file.
fn has_typ_files_in(dir: &PathBuf) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in rd {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "typ" {
                    return true;
                }
            }
        }
    }
    false
}

/// Resolve the user's home directory.
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}
