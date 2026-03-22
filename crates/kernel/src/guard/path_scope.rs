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

//! Path-scope guard for restricting file-access tools to a workspace directory.
//!
//! Ensures that file-manipulating tools (read, write, edit, grep, etc.) only
//! operate within the configured workspace root or explicitly whitelisted
//! paths. Paths outside scope are blocked and routed to human approval.
//!
//! # Scope
//!
//! This guard only intercepts **structured file-access tools** (read-file,
//! write-file, etc.). It does **not** inspect `bash` / `shell_exec` commands,
//! so a `cat /etc/passwd` via bash bypasses this layer entirely. Shell tools
//! are gated by Layers 1–2 (taint + pattern) and the approval policy instead.
//! Defense-in-depth means no single layer is expected to catch everything.

use std::{
    path::{Component, Path, PathBuf},
    sync::RwLock,
};

/// Tools that use a `file_path` parameter.
///
/// SYNC: when adding a new file-access tool to the registry
/// (`crates/app/src/tools/mod.rs`), add it here too.
pub const FILE_PATH_TOOLS: &[&str] = &["read-file", "write-file", "edit-file"];
// TODO: `multi-edit` uses `edits[].file_path` and `file-stats` uses `paths[]`
// (arrays of paths). The guard currently only checks a single top-level param,
// so neither tool is covered here yet. Add array-aware path checking in a
// follow-up.

/// Tools that use a `path` parameter.
///
/// SYNC: when adding a new file-access tool to the registry
/// (`crates/app/src/tools/mod.rs`), add it here too.
pub const PATH_TOOLS: &[&str] = &["grep", "list-directory", "find-files", "walk-directory"];

/// Guard that restricts file-access tools to a workspace directory and optional
/// whitelist entries.
///
/// Returns `None` (pass) when the resolved path is within scope, or
/// `Some(reason)` when the path escapes the allowed boundaries.
///
/// # Security Limitations
///
/// - **Symlinks**: This guard uses **lexical** path normalization (no
///   filesystem access). Symlinks inside the workspace pointing to external
///   paths will not be detected (tracked in issue #584).
/// - **Case sensitivity**: On macOS/Windows (case-insensitive filesystems),
///   path comparison is lowercased to prevent case-variant bypasses.
pub struct PathScopeGuard {
    workspace:         PathBuf,
    whitelist:         Vec<PathBuf>,
    /// Paths approved by the user at runtime. Persists for the lifetime of the
    /// guard (i.e. the process) and is cleared on restart.
    approved_prefixes: RwLock<Vec<PathBuf>>,
}

impl PathScopeGuard {
    /// Create a new path-scope guard.
    ///
    /// `workspace` is the primary allowed root directory.
    /// `whitelist` contains additional allowed path prefixes.
    pub fn new(workspace: PathBuf, whitelist: Vec<PathBuf>) -> Self {
        Self {
            workspace:         normalize_path(&workspace),
            whitelist:         whitelist.iter().map(|p| normalize_path(p)).collect(),
            approved_prefixes: RwLock::new(Vec::new()),
        }
    }

    /// Check whether a tool invocation accesses a path within the allowed
    /// scope.
    ///
    /// Returns `None` if the tool is not a file-access tool or the path is
    /// within scope. Returns `Some(reason)` if the path escapes the workspace
    /// and all whitelist entries.
    pub fn check(&self, tool_name: &str, args: &serde_json::Value) -> Option<String> {
        let param_name = if FILE_PATH_TOOLS.contains(&tool_name) {
            "file_path"
        } else if PATH_TOOLS.contains(&tool_name) {
            "path"
        } else {
            // Not a file-access tool — pass through.
            return None;
        };

        let raw_path = match args.get(param_name).and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                // Missing or non-string path arg — let the tool itself handle
                // validation. Log a warning so this is visible in traces.
                tracing::warn!(
                    tool = tool_name,
                    param = param_name,
                    "path-scope guard: file tool missing expected path param"
                );
                return None;
            }
        };

        let resolved = if Path::new(raw_path).is_absolute() {
            normalize_path(Path::new(raw_path))
        } else {
            // Relative paths are resolved against the workspace root.
            normalize_path(&self.workspace.join(raw_path))
        };

        tracing::debug!(
            tool = tool_name,
            raw_path = raw_path,
            path = %resolved.display(),
            workspace = %self.workspace.display(),
            "path-scope guard checking file access"
        );

        if path_starts_with(&resolved, &self.workspace) {
            return None;
        }

        for allowed in &self.whitelist {
            if path_starts_with(&resolved, allowed) {
                return None;
            }
        }

        // For directory-scanning tools, also allow paths that are *ancestors* of the
        // workspace or a whitelist entry. Example: `grep path:"/Users/foo/.config"`
        // when workspace is `/Users/foo/.config/rara/workspace`. The tool will
        // search the entire subtree of this ancestor path, which includes (but is
        // not limited to) the workspace.
        //
        // Excluded: root path `/` (1 component) — would trivially match everything.
        // Excluded: file-targeting tools (read-file, write-file, edit-file) — they
        // operate on specific files, not directory trees.
        // Depth-limited: ancestors more than MAX_ANCESTOR_DEPTH levels above the
        // workspace/whitelist entry are rejected to prevent overly broad scans.
        const MAX_ANCESTOR_DEPTH: usize = 3;
        let ancestor_components = resolved.components().count();
        if PATH_TOOLS.contains(&tool_name) && ancestor_components > 1 {
            let ws_components = self.workspace.components().count();
            if ws_components.saturating_sub(ancestor_components) <= MAX_ANCESTOR_DEPTH
                && path_starts_with(&self.workspace, &resolved)
            {
                tracing::debug!(
                    path = %resolved.display(),
                    workspace = %self.workspace.display(),
                    depth = ws_components.saturating_sub(ancestor_components),
                    "path-scope guard: path is ancestor of workspace, allowing"
                );
                return None;
            }
            for allowed in &self.whitelist {
                let allowed_components = allowed.components().count();
                if allowed_components.saturating_sub(ancestor_components) <= MAX_ANCESTOR_DEPTH
                    && path_starts_with(allowed, &resolved)
                {
                    tracing::debug!(
                        path = %resolved.display(),
                        whitelist_entry = %allowed.display(),
                        depth = allowed_components.saturating_sub(ancestor_components),
                        "path-scope guard: path is ancestor of whitelist entry, allowing"
                    );
                    return None;
                }
            }
        }

        // Check user-approved paths from earlier approvals in this session.
        // Fail-closed: if the lock is poisoned we skip dynamic approvals and
        // fall through to the "blocked" path, which is the safe default.
        match self.approved_prefixes.read() {
            Ok(approved) => {
                for prefix in approved.iter() {
                    if path_starts_with(&resolved, prefix) {
                        tracing::debug!(
                            path = %resolved.display(),
                            approved_prefix = %prefix.display(),
                            "path allowed by dynamic approval"
                        );
                        return None;
                    }
                }
            }
            Err(_) => {
                tracing::warn!(
                    "approved_prefixes lock poisoned, skipping dynamic approvals (fail-closed)"
                );
            }
        }

        Some(format!(
            "path '{}' is outside workspace '{}' and not in whitelist",
            resolved.display(),
            self.workspace.display(),
        ))
    }

    /// Record a user-approved path so that subsequent accesses to the same
    /// directory tree pass without re-prompting.
    ///
    /// Extracts the directory prefix from the tool's path argument and adds it
    /// to the dynamic whitelist. For file tools (`read-file`, etc.) the parent
    /// directory is whitelisted; for directory tools (`list-directory`, etc.)
    /// the path itself is whitelisted.
    pub fn approve_path(&self, tool_name: &str, args: &serde_json::Value) {
        let param_name = if FILE_PATH_TOOLS.contains(&tool_name) {
            "file_path"
        } else if PATH_TOOLS.contains(&tool_name) {
            "path"
        } else {
            return;
        };

        let raw_path = match args.get(param_name).and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return,
        };

        let resolved = if Path::new(raw_path).is_absolute() {
            normalize_path(Path::new(raw_path))
        } else {
            normalize_path(&self.workspace.join(raw_path))
        };

        // For file tools, whitelist the parent directory so sibling files are
        // also covered. For directory tools, whitelist the path itself.
        let prefix = if FILE_PATH_TOOLS.contains(&tool_name) {
            resolved
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or(resolved)
        } else {
            resolved
        };

        let mut approved = match self.approved_prefixes.write() {
            Ok(guard) => guard,
            Err(e) => e.into_inner(),
        };

        // Skip if already covered by an existing approved prefix.
        if approved
            .iter()
            .any(|existing| path_starts_with(&prefix, existing))
        {
            return;
        }

        // Prune narrower entries that are now covered by the new broader prefix.
        approved.retain(|existing| !path_starts_with(existing, &prefix));

        tracing::info!(
            prefix = %prefix.display(),
            "adding user-approved path prefix to dynamic whitelist"
        );
        approved.push(prefix);
    }
}

/// Case-aware `starts_with` check.
///
/// On case-insensitive filesystems (macOS, Windows) this lowercases both paths
/// before comparison to prevent bypasses like `/Users/Ryan/..` vs
/// `/Users/ryan/..`. On Linux, uses the standard component-aware comparison.
fn path_starts_with(path: &Path, base: &Path) -> bool {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        // Lowercase both sides and compare component-by-component.
        let path_lower = PathBuf::from(path.to_string_lossy().to_lowercase());
        let base_lower = PathBuf::from(base.to_string_lossy().to_lowercase());
        path_lower.starts_with(&base_lower)
    } else {
        path.starts_with(base)
    }
}

/// Normalize a path lexically by resolving `.` and `..` components without
/// touching the filesystem. This is intentional — the guard must work on paths
/// that may not yet exist.
///
/// Expects an **absolute** path. For relative paths, `..` components that
/// escape the root are preserved as-is, which is likely not what you want.
/// Callers should join relative paths to an absolute root before calling this.
pub fn normalize_path(path: &Path) -> PathBuf {
    debug_assert!(
        path.is_absolute(),
        "normalize_path expects absolute paths, got: {}",
        path.display()
    );
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {
                // Skip `.`
            }
            Component::ParentDir => {
                // Go up one level, but never pop past the root.
                // NOTE: For pure relative paths (empty accumulator), the `..`
                // is preserved as-is. This is safe in `check()` because
                // relative paths are always joined to the workspace (absolute)
                // before normalization, so the accumulator is never empty
                // there. Keep this in mind if reusing this function elsewhere.
                if !out.pop() {
                    out.push(component);
                }
            }
            other => {
                out.push(other);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn guard() -> PathScopeGuard {
        PathScopeGuard::new(PathBuf::from("/home/user/project"), vec![])
    }

    fn guard_with_whitelist() -> PathScopeGuard {
        PathScopeGuard::new(
            PathBuf::from("/home/user/project"),
            vec![PathBuf::from("/tmp/scratch")],
        )
    }

    // ── normalize_path ──────────────────────────────────────────────

    #[test]
    fn normalize_collapses_dotdot() {
        assert_eq!(
            normalize_path(Path::new("/home/user/project/src/../Cargo.toml")),
            PathBuf::from("/home/user/project/Cargo.toml")
        );
    }

    #[test]
    fn normalize_collapses_dot() {
        assert_eq!(
            normalize_path(Path::new("/home/user/./project/./src")),
            PathBuf::from("/home/user/project/src")
        );
    }

    #[test]
    fn normalize_does_not_go_past_root() {
        assert_eq!(
            normalize_path(Path::new("/../../etc/passwd")),
            PathBuf::from("/etc/passwd")
        );
    }

    #[test]
    fn normalize_absolute_with_multiple_dotdot() {
        assert_eq!(
            normalize_path(Path::new("/a/b/c/../../d")),
            PathBuf::from("/a/d")
        );
    }

    // ── non-file tools pass through ─────────────────────────────────

    #[test]
    fn non_file_tool_passes() {
        let g = guard();
        assert_eq!(g.check("bash", &json!({"command": "rm -rf /"})), None);
    }

    #[test]
    fn unknown_tool_passes() {
        let g = guard();
        assert_eq!(
            g.check("web_fetch", &json!({"url": "http://evil.com"})),
            None
        );
    }

    // ── missing path arg passes ─────────────────────────────────────

    #[test]
    fn missing_path_arg_passes() {
        let g = guard();
        assert_eq!(g.check("read-file", &json!({})), None);
    }

    // ── absolute paths within workspace pass ────────────────────────

    #[test]
    fn absolute_path_inside_workspace_passes() {
        let g = guard();
        let args = json!({"file_path": "/home/user/project/src/main.rs"});
        assert_eq!(g.check("read-file", &args), None);
    }

    #[test]
    fn workspace_root_itself_passes() {
        let g = guard();
        let args = json!({"file_path": "/home/user/project"});
        assert_eq!(g.check("write-file", &args), None);
    }

    // ── relative paths resolved against workspace ───────────────────

    #[test]
    fn relative_path_inside_workspace_passes() {
        let g = guard();
        let args = json!({"file_path": "src/main.rs"});
        assert_eq!(g.check("read-file", &args), None);
    }

    #[test]
    fn relative_dotdot_staying_inside_passes() {
        let g = guard();
        let args = json!({"file_path": "src/../Cargo.toml"});
        assert_eq!(g.check("edit-file", &args), None);
    }

    // ── absolute paths outside workspace blocked ────────────────────

    #[test]
    fn absolute_path_outside_workspace_blocked() {
        let g = guard();
        let args = json!({"file_path": "/etc/passwd"});
        let result = g.check("read-file", &args);
        assert!(result.is_some());
        assert!(result.unwrap().contains("outside workspace"));
    }

    #[test]
    fn dotdot_escape_blocked() {
        let g = guard();
        let args = json!({"file_path": "/home/user/project/../../etc/passwd"});
        let result = g.check("read-file", &args);
        assert!(result.is_some());
    }

    #[test]
    fn relative_dotdot_escape_blocked() {
        let g = guard();
        let args = json!({"file_path": "../../../etc/passwd"});
        let result = g.check("read-file", &args);
        assert!(result.is_some());
    }

    // ── tool-specific param extraction ──────────────────────────────

    #[test]
    fn grep_uses_path_param() {
        let g = guard();
        let args = json!({"path": "/etc/shadow", "pattern": "root"});
        assert!(g.check("grep", &args).is_some());
    }

    #[test]
    fn list_directory_uses_path_param() {
        let g = guard();
        let args = json!({"path": "/home/user/project/src"});
        assert_eq!(g.check("list-directory", &args), None);
    }

    #[test]
    fn find_files_uses_path_param() {
        let g = guard();
        let args = json!({"path": "/secret"});
        assert!(g.check("find-files", &args).is_some());
    }

    // ── whitelist ───────────────────────────────────────────────────

    #[test]
    fn whitelisted_path_passes() {
        let g = guard_with_whitelist();
        let args = json!({"file_path": "/tmp/scratch/output.txt"});
        assert_eq!(g.check("write-file", &args), None);
    }

    #[test]
    fn whitelist_root_itself_passes() {
        let g = guard_with_whitelist();
        let args = json!({"path": "/tmp/scratch"});
        assert_eq!(g.check("list-directory", &args), None);
    }

    #[test]
    fn path_outside_both_workspace_and_whitelist_blocked() {
        let g = guard_with_whitelist();
        let args = json!({"file_path": "/var/secret"});
        assert!(g.check("read-file", &args).is_some());
    }

    #[test]
    fn multiple_whitelist_entries_all_pass() {
        let g = PathScopeGuard::new(
            PathBuf::from("/home/user/project"),
            vec![
                PathBuf::from("/tmp/scratch"),
                PathBuf::from("/var/log/rara"),
                PathBuf::from("/home/user/.claude"),
            ],
        );
        // Each whitelist entry allows its subtree.
        assert_eq!(
            g.check("read-file", &json!({"file_path": "/var/log/rara/rara.log"})),
            None
        );
        assert_eq!(
            g.check(
                "list-directory",
                &json!({"path": "/home/user/.claude/skills"})
            ),
            None
        );
        // Unrelated path still blocked.
        assert!(
            g.check("read-file", &json!({"file_path": "/etc/secret"}))
                .is_some()
        );
    }

    // ── dynamic approval ────────────────────────────────────────────

    #[test]
    fn approved_directory_passes_subsequent_checks() {
        let g = guard();
        let dir_args = json!({"path": "/tmp/sanyuan-skills"});
        // First check blocks.
        assert!(g.check("list-directory", &dir_args).is_some());
        // Simulate user approval.
        g.approve_path("list-directory", &dir_args);
        // Same path now passes.
        assert_eq!(g.check("list-directory", &dir_args), None);
        // Sub-path also passes.
        let sub_args = json!({"path": "/tmp/sanyuan-skills/skills"});
        assert_eq!(g.check("list-directory", &sub_args), None);
        // File under the tree also passes.
        let file_args = json!({"file_path": "/tmp/sanyuan-skills/skills/sigma/SKILL.md"});
        assert_eq!(g.check("read-file", &file_args), None);
    }

    #[test]
    fn approved_file_whitelists_parent_directory() {
        let g = guard();
        let file_args = json!({"file_path": "/tmp/data/report.txt"});
        assert!(g.check("read-file", &file_args).is_some());
        g.approve_path("read-file", &file_args);
        // Sibling file in same directory passes.
        let sibling = json!({"file_path": "/tmp/data/other.txt"});
        assert_eq!(g.check("read-file", &sibling), None);
    }

    #[test]
    fn approval_does_not_overshoot() {
        let g = guard();
        let dir_args = json!({"path": "/tmp/sanyuan-skills"});
        g.approve_path("list-directory", &dir_args);
        // Unrelated path still blocked.
        let other = json!({"file_path": "/var/secret"});
        assert!(g.check("read-file", &other).is_some());
    }

    #[test]
    fn duplicate_approval_is_deduplicated() {
        let g = guard();
        let args = json!({"path": "/tmp/test"});
        g.approve_path("list-directory", &args);
        g.approve_path("list-directory", &args);
        let approved = g.approved_prefixes.read().unwrap();
        assert_eq!(approved.len(), 1);
    }

    #[test]
    fn sub_path_approval_deduped_by_parent() {
        let g = guard();
        // Approve a parent first.
        g.approve_path("list-directory", &json!({"path": "/tmp/root"}));
        // Approving a sub-path should be a no-op (already covered).
        g.approve_path("list-directory", &json!({"path": "/tmp/root/sub"}));
        let approved = g.approved_prefixes.read().unwrap();
        assert_eq!(approved.len(), 1);
    }

    #[test]
    fn broader_prefix_prunes_narrower_entries() {
        let g = guard();
        // Approve a narrow path first.
        g.approve_path("list-directory", &json!({"path": "/tmp/root/sub"}));
        assert_eq!(g.approved_prefixes.read().unwrap().len(), 1);
        // Approve a broader parent — should replace the narrow entry.
        g.approve_path("list-directory", &json!({"path": "/tmp/root"}));
        let approved = g.approved_prefixes.read().unwrap();
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0], PathBuf::from("/tmp/root"));
        // Sub-path still works.
        let sub_args = json!({"path": "/tmp/root/sub/deep"});
        assert_eq!(g.check("list-directory", &sub_args), None);
    }

    // ── edge cases ──────────────────────────────────────────────────

    #[test]
    fn prefix_collision_blocked() {
        // "/home/user/project-evil" should NOT pass for workspace "/home/user/project"
        // because `starts_with` on `PathBuf` checks component boundaries.
        let g = guard();
        let args = json!({"file_path": "/home/user/project-evil/exploit.sh"});
        assert!(g.check("read-file", &args).is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn case_variant_inside_workspace_passes() {
        // macOS is case-insensitive: /Home/User/Project == /home/user/project
        let g = guard();
        let args = json!({"file_path": "/Home/User/Project/src/main.rs"});
        assert_eq!(g.check("read-file", &args), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn case_variant_outside_workspace_blocked() {
        let g = guard();
        let args = json!({"file_path": "/Home/User/OTHER/secret.txt"});
        assert!(g.check("read-file", &args).is_some());
    }

    #[test]
    fn workspace_with_trailing_dotdot_normalized() {
        // Workspace itself may contain `..` — it gets normalized in `new()`.
        let g = PathScopeGuard::new(PathBuf::from("/home/user/project/../project"), vec![]);
        let args = json!({"file_path": "/home/user/project/src/lib.rs"});
        assert_eq!(g.check("read-file", &args), None);
    }

    // ── ancestor path access ────────────────────────────────────────

    #[test]
    fn ancestor_of_workspace_passes_for_path_tools() {
        let g = guard(); // workspace = /home/user/project
        // /home/user is a parent of /home/user/project
        let args = json!({"path": "/home/user"});
        assert_eq!(g.check("grep", &args), None);
        assert_eq!(g.check("list-directory", &args), None);
        assert_eq!(g.check("find-files", &args), None);
    }

    #[test]
    fn ancestor_of_workspace_blocked_for_file_tools() {
        let g = guard();
        // Same ancestor path, but file tools should NOT get the ancestor exception
        let args = json!({"file_path": "/home/user"});
        assert!(g.check("read-file", &args).is_some());
    }

    #[test]
    fn grandparent_of_workspace_passes() {
        let g = guard();
        let args = json!({"path": "/home"});
        assert_eq!(g.check("grep", &args), None);
    }

    #[test]
    fn root_path_blocked_despite_being_ancestor() {
        let g = guard();
        let args = json!({"path": "/"});
        assert!(g.check("grep", &args).is_some());
    }

    #[test]
    fn ancestor_of_whitelist_entry_passes() {
        let g = guard_with_whitelist(); // whitelist = /tmp/scratch
        let args = json!({"path": "/tmp"});
        assert_eq!(g.check("list-directory", &args), None);
    }

    #[test]
    fn unrelated_sibling_of_workspace_parent_blocked() {
        let g = guard(); // workspace = /home/user/project
        let args = json!({"path": "/home/other"});
        assert!(g.check("grep", &args).is_some());
    }

    #[test]
    fn common_prefix_not_ancestor_blocked() {
        let g = guard(); // workspace = /home/user/project
        // /home/us is NOT an ancestor of /home/user/project (component boundary)
        let args = json!({"path": "/home/us"});
        assert!(g.check("grep", &args).is_some());
    }

    #[test]
    fn ancestor_beyond_max_depth_blocked() {
        // workspace = /a/b/c/d/e/f (7 components). /a/b is 3 components.
        // depth = 7 - 3 = 4 > MAX_ANCESTOR_DEPTH (3) → blocked.
        let g = PathScopeGuard::new(PathBuf::from("/a/b/c/d/e/f"), vec![]);
        let args = json!({"path": "/a/b"});
        assert!(g.check("grep", &args).is_some());
    }

    #[test]
    fn ancestor_at_max_depth_passes() {
        // workspace = /a/b/c/d/e/f (7 components). /a/b/c is 4 components.
        // depth = 7 - 4 = 3 == MAX_ANCESTOR_DEPTH → passes.
        let g = PathScopeGuard::new(PathBuf::from("/a/b/c/d/e/f"), vec![]);
        let args = json!({"path": "/a/b/c"});
        assert_eq!(g.check("grep", &args), None);
    }
}
