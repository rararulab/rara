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

use std::path::{Component, Path, PathBuf};

/// Tools that use a `file_path` parameter.
///
/// SYNC: when adding a new file-access tool to the registry
/// (`crates/app/src/tools/mod.rs`), add it here too.
pub const FILE_PATH_TOOLS: &[&str] = &["read-file", "write-file", "edit-file"];

/// Tools that use a `path` parameter.
///
/// SYNC: when adding a new file-access tool to the registry
/// (`crates/app/src/tools/mod.rs`), add it here too.
pub const PATH_TOOLS: &[&str] = &["grep", "list-directory", "find-files"];

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
    workspace: PathBuf,
    whitelist: Vec<PathBuf>,
}

impl PathScopeGuard {
    /// Create a new path-scope guard.
    ///
    /// `workspace` is the primary allowed root directory.
    /// `whitelist` contains additional allowed path prefixes.
    pub fn new(workspace: PathBuf, whitelist: Vec<PathBuf>) -> Self {
        Self {
            workspace: normalize_path(&workspace),
            whitelist: whitelist.iter().map(|p| normalize_path(p)).collect(),
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

        Some(format!(
            "path '{}' is outside workspace '{}' and not in whitelist",
            resolved.display(),
            self.workspace.display(),
        ))
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
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
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
}
