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

use std::path::{Component, Path, PathBuf};

/// Guard that restricts file-access tools to a workspace directory and optional
/// whitelist entries.
///
/// Returns `None` (pass) when the resolved path is within scope, or
/// `Some(reason)` when the path escapes the allowed boundaries.
///
/// # Security Limitations
///
/// This guard uses **lexical** path normalization (no filesystem access).
/// Symlinks inside the workspace pointing to external paths will not be
/// detected. If the threat model includes symlink-based escapes, consider
/// adding an optional `std::fs::canonicalize` pass for paths that exist on
/// disk (tracked as a follow-up).
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
        // SYNC: keep in sync with tool registry (crates/app/src/tools/mod.rs)
        let param_name = match tool_name {
            "read-file" | "write-file" | "edit-file" => "file_path",
            "grep" | "list-directory" | "find-files" => "path",
            // Not a file-access tool — pass through.
            _ => return None,
        };

        let raw_path = match args.get(param_name).and_then(|v| v.as_str()) {
            Some(p) => p,
            // Missing path arg — let the tool itself handle validation.
            None => return None,
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

        if resolved.starts_with(&self.workspace) {
            return None;
        }

        for allowed in &self.whitelist {
            if resolved.starts_with(allowed) {
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

/// Normalize a path lexically by resolving `.` and `..` components without
/// touching the filesystem. This is intentional — the guard must work on paths
/// that may not yet exist.
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {
                // Skip `.`
            }
            Component::ParentDir => {
                // Go up one level, but never pop past the root.
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
    fn normalize_relative_path() {
        assert_eq!(
            normalize_path(Path::new("src/../lib.rs")),
            PathBuf::from("lib.rs")
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

    #[test]
    fn workspace_with_trailing_dotdot_normalized() {
        // Workspace itself may contain `..` — it gets normalized in `new()`.
        let g = PathScopeGuard::new(PathBuf::from("/home/user/project/../project"), vec![]);
        let args = json!({"file_path": "/home/user/project/src/lib.rs"});
        assert_eq!(g.check("read-file", &args), None);
    }
}
