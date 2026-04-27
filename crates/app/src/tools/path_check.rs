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

//! Path validation for write-class file tools.
//!
//! All write-side tools (`write-file`, `edit-file`, `multi-edit`,
//! `delete-file`, `create-directory`) call [`resolve_writable`] before
//! touching the filesystem. The check defeats two escape vectors that the
//! retired lexical `PathScopeGuard` could not (#1936):
//!
//! 1. **Absolute path escape** — a raw path outside `workspace_dir()` is
//!    rejected.
//! 2. **Symlink escape** — paths are resolved with [`tokio::fs::canonicalize`]
//!    so a symlink inside the workspace pointing at `/etc` is detected.
//!
//! Files that do not yet exist (e.g. `write-file` creating a new file) are
//! handled by canonicalising the **parent directory** instead, then joining
//! the leaf name back on. The leaf is restricted to a single component so
//! `..` cannot reappear post-canonicalise.
//!
//! Read-side tools intentionally bypass this helper — see the rationale in
//! `crates/app/src/tools/AGENT.md` and #1936.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, anyhow};

/// Resolve `raw` to an absolute path that is guaranteed to live inside
/// `rara_paths::workspace_dir()`, with all symlinks resolved.
///
/// Relative paths are joined to the workspace before resolution, matching
/// the existing tool conventions.
pub async fn resolve_writable(raw: &str) -> anyhow::Result<PathBuf> {
    resolve_writable_in(rara_paths::workspace_dir(), raw).await
}

/// Same as [`resolve_writable`] but with the workspace path injected, so
/// tests can use a tempdir without touching the global `OnceLock`.
async fn resolve_writable_in(workspace: &Path, raw: &str) -> anyhow::Result<PathBuf> {
    let workspace_canon = canonicalize_workspace(workspace).await?;

    let joined = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        workspace.join(raw)
    };

    let resolved = match tokio::fs::canonicalize(&joined).await {
        Ok(p) => p,
        Err(_) => {
            // Path itself does not exist yet — canonicalise the parent and
            // re-attach a single, validated leaf component.
            let parent = joined
                .parent()
                .ok_or_else(|| anyhow!("path '{raw}' has no parent"))?;
            let leaf = joined
                .file_name()
                .ok_or_else(|| anyhow!("path '{raw}' has no file name"))?;

            // The leaf must be a single normal component — no `..`, no
            // separators — so post-canonicalise concatenation cannot
            // re-escape.
            let leaf_path = Path::new(leaf);
            if leaf_path.components().count() != 1
                || !matches!(leaf_path.components().next(), Some(Component::Normal(_)))
            {
                return Err(anyhow!("path '{raw}' has an invalid leaf component"));
            }

            // The parent might also not exist yet (e.g. `write-file` creating
            // nested directories). Walk up until we find an existing
            // ancestor, canonicalise it, then re-append the relative tail.
            let (anchor_canon, tail) = canonicalize_existing_ancestor(parent).await?;
            anchor_canon.join(tail).join(leaf)
        }
    };

    if !path_starts_with(&resolved, &workspace_canon) {
        return Err(anyhow!(
            "path '{}' resolves outside workspace '{}'",
            resolved.display(),
            workspace_canon.display()
        ));
    }
    Ok(resolved)
}

/// Canonicalize the workspace once, with a friendly error message.
async fn canonicalize_workspace(workspace: &Path) -> anyhow::Result<PathBuf> {
    tokio::fs::canonicalize(workspace).await.context(format!(
        "failed to canonicalize workspace '{}'",
        workspace.display()
    ))
}

/// Walk up `path`, returning the first existing ancestor (canonicalised) and
/// the relative tail that was missing.
async fn canonicalize_existing_ancestor(path: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let mut tail = PathBuf::new();
    let mut cursor = path.to_path_buf();
    loop {
        match tokio::fs::canonicalize(&cursor).await {
            Ok(canon) => return Ok((canon, tail)),
            Err(_) => {
                let leaf = cursor
                    .file_name()
                    .ok_or_else(|| anyhow!("no existing ancestor for '{}'", path.display()))?
                    .to_owned();
                let parent = cursor
                    .parent()
                    .ok_or_else(|| anyhow!("no existing ancestor for '{}'", path.display()))?
                    .to_path_buf();
                tail = Path::new(&leaf).join(&tail);
                cursor = parent;
            }
        }
    }
}

/// Case-aware `starts_with` that lowercases on macOS / Windows where the
/// underlying filesystem is typically case-insensitive.
fn path_starts_with(path: &Path, base: &Path) -> bool {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        let path_lower = PathBuf::from(path.to_string_lossy().to_lowercase());
        let base_lower = PathBuf::from(base.to_string_lossy().to_lowercase());
        path_lower.starts_with(&base_lower)
    } else {
        path.starts_with(base)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    /// `resolve_writable_in` joins relative paths under the workspace.
    #[tokio::test]
    async fn relative_path_joins_workspace() {
        let workspace = TempDir::new().expect("tempdir");
        let target = workspace.path().join("file.txt");
        tokio::fs::write(&target, b"x").await.unwrap();
        let resolved = resolve_writable_in(workspace.path(), "file.txt")
            .await
            .expect("resolve");
        let workspace_canon = tokio::fs::canonicalize(workspace.path()).await.unwrap();
        assert!(resolved.starts_with(workspace_canon));
    }

    /// `/etc/passwd` is rejected.
    #[tokio::test]
    async fn absolute_outside_workspace_rejected() {
        let workspace = TempDir::new().expect("tempdir");
        let err = resolve_writable_in(workspace.path(), "/etc/passwd")
            .await
            .expect_err("must reject");
        assert!(err.to_string().contains("outside workspace"));
    }

    /// A symlink inside the workspace pointing at `/etc` is rejected.
    #[tokio::test]
    async fn symlink_escape_rejected() {
        let workspace = TempDir::new().expect("workspace tempdir");
        let outside = TempDir::new().expect("outside tempdir");
        let outside_file = outside.path().join("victim.txt");
        tokio::fs::write(&outside_file, b"secret").await.unwrap();

        let link = workspace.path().join("escape");
        symlink(&outside_file, &link).expect("symlink");

        let err = resolve_writable_in(workspace.path(), &link.to_string_lossy())
            .await
            .expect_err("symlink escape must be rejected");
        assert!(err.to_string().contains("outside workspace"));
    }
}
