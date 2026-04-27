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
    let workspace = rara_paths::workspace_dir().clone();
    let workspace_canon = canonicalize_workspace(&workspace).await?;

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

    /// `resolve_writable` joins relative paths under the workspace.
    #[tokio::test]
    async fn relative_path_joins_workspace() {
        // Best-effort: the workspace dir is a global OnceLock, so we just
        // verify the resolution stays within it.
        let workspace = rara_paths::workspace_dir().clone();
        // Use a file we KNOW exists inside workspace by creating one.
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let target = workspace.join("__rara_path_check_test__.txt");
        tokio::fs::write(&target, b"x").await.unwrap();
        let resolved = resolve_writable("__rara_path_check_test__.txt")
            .await
            .expect("resolve");
        assert!(resolved.starts_with(tokio::fs::canonicalize(&workspace).await.unwrap()));
        let _ = tokio::fs::remove_file(&target).await;
    }

    /// `/etc/passwd` is rejected.
    #[tokio::test]
    async fn absolute_outside_workspace_rejected() {
        let err = resolve_writable("/etc/passwd")
            .await
            .expect_err("must reject");
        assert!(err.to_string().contains("outside workspace"));
    }

    /// A symlink inside the workspace pointing at `/etc` is rejected.
    #[tokio::test]
    async fn symlink_escape_rejected() {
        // Create a symlink target outside the workspace via a temp dir.
        let workspace = rara_paths::workspace_dir().clone();
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let outside = TempDir::new().expect("tempdir");
        let outside_file = outside.path().join("victim.txt");
        tokio::fs::write(&outside_file, b"secret").await.unwrap();

        let link = workspace.join("__rara_path_check_symlink__");
        // Clean up any stale link from a prior failed run.
        let _ = tokio::fs::remove_file(&link).await;
        symlink(&outside_file, &link).expect("symlink");

        let err = resolve_writable(&link.to_string_lossy())
            .await
            .expect_err("symlink escape must be rejected");
        assert!(err.to_string().contains("outside workspace"));

        let _ = tokio::fs::remove_file(&link).await;
    }
}
