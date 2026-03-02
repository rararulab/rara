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

//! PathGuard — file-system access control for sandboxed agent processes.
//!
//! Enforces the [`SandboxConfig`] rules:
//! 1. **Denied paths** always reject (highest priority).
//! 2. **Read-only paths** allow reads, block writes.
//! 3. **Allowed paths** allow both reads and writes.
//! 4. **Workspace** is always read/write accessible.
//! 5. Everything else is denied.
//!
//! Path matching uses canonicalized prefix comparison. A path is "under" a
//! configured prefix if its canonical form starts with the prefix's canonical
//! form. This prevents symlink and `..` traversal attacks.

use std::{
    path::{Path, PathBuf},
    sync::RwLock,
};

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    error::KernelError,
    guard::{Guard, GuardContext, Verdict},
    process::SandboxConfig,
};

/// File-system access guard that enforces [`SandboxConfig`] rules.
///
/// Wraps an inner [`Guard`] implementation: path checks run first, then
/// the inner guard is consulted for non-path-related decisions.
pub struct PathGuard {
    config:    RwLock<SandboxConfig>,
    workspace: PathBuf,
    inner:     Box<dyn Guard>,
}

impl PathGuard {
    /// Create a new `PathGuard` wrapping an existing guard.
    ///
    /// The `workspace` path is always allowed for read/write access.
    pub fn new(config: SandboxConfig, workspace: PathBuf, inner: Box<dyn Guard>) -> Self {
        Self {
            config: RwLock::new(config),
            workspace,
            inner,
        }
    }

    /// Update the sandbox config at runtime (called by settings subscriber).
    pub fn update_config(&self, new_config: SandboxConfig) {
        *self.config.write().unwrap() = new_config;
    }

    /// Check if a path is allowed for the given operation.
    ///
    /// # Arguments
    /// - `path` — the file path to check (will be canonicalized)
    /// - `write` — `true` for write operations, `false` for read-only
    ///
    /// # Precedence
    /// 1. Denied paths always reject.
    /// 2. Read-only paths allow reads, block writes.
    /// 3. Allowed paths allow both reads and writes.
    /// 4. Workspace directory is always allowed.
    /// 5. Everything else is denied.
    pub fn check_access(&self, path: &Path, write: bool) -> Result<(), KernelError> {
        let config = self.config.read().unwrap();

        // Normalize the path: try to resolve it relative to known existing
        // prefixes so that non-existent child paths still compare correctly
        // on systems with symlinked temp dirs (e.g., macOS /var -> /private/var).
        let canonical = resolve_against_parents(path);

        // 1. Denied paths take highest precedence.
        if self.matches_any_paths(&canonical, &config.denied_paths) {
            return Err(KernelError::SandboxAccessDenied {
                path:      path.display().to_string(),
                operation: if write { "write" } else { "read" }.to_string(),
            });
        }

        // 2. Read-only paths: reads OK, writes denied.
        if self.matches_any_paths(&canonical, &config.read_only_paths) {
            if write {
                return Err(KernelError::SandboxAccessDenied {
                    path:      path.display().to_string(),
                    operation: "write".to_string(),
                });
            }
            return Ok(());
        }

        // 3. Allowed paths: both reads and writes OK.
        if self.matches_any_paths(&canonical, &config.allowed_paths) {
            return Ok(());
        }

        // 4. Workspace is always allowed.
        let workspace_canonical = canonicalize_or_normalize(&self.workspace);
        if canonical.starts_with(&workspace_canonical) {
            return Ok(());
        }

        // 5. Default: deny.
        Err(KernelError::SandboxAccessDenied {
            path:      path.display().to_string(),
            operation: if write { "write" } else { "read" }.to_string(),
        })
    }

    /// Resolve a relative path against the workspace directory.
    ///
    /// Returns an error if the resolved path escapes the workspace (path
    /// traversal).
    pub fn resolve(&self, path: &str) -> Result<PathBuf, KernelError> {
        let resolved = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.workspace.join(path)
        };

        let canonical = resolve_against_parents(&resolved);
        let workspace_canonical = canonicalize_or_normalize(&self.workspace);

        if canonical.starts_with(&workspace_canonical) {
            Ok(canonical)
        } else {
            Err(KernelError::SandboxPathError {
                message: format!(
                    "path '{}' escapes workspace '{}'",
                    path,
                    self.workspace.display()
                ),
            })
        }
    }

    /// Check if a canonical path starts with any of the configured prefixes.
    fn matches_any_paths(&self, canonical: &Path, prefixes: &[String]) -> bool {
        prefixes.iter().any(|prefix| {
            let prefix_canonical = resolve_against_parents(Path::new(prefix));
            canonical.starts_with(&prefix_canonical)
        })
    }
}

/// Extract file path arguments from tool call arguments.
///
/// Inspects known tool names and extracts path arguments for sandbox
/// checking.
///
/// Returns a list of `(path, is_write)` tuples.
fn extract_file_paths(tool_name: &str, args: &Value) -> Vec<(String, bool)> {
    let mut paths = Vec::new();

    match tool_name {
        "file_read" | "read_file" | "cat" => {
            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                paths.push((path.to_string(), false));
            }
            if let Some(path) = args.get("file_path").and_then(|v| v.as_str()) {
                paths.push((path.to_string(), false));
            }
        }
        "file_write" | "write_file" | "write" => {
            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                paths.push((path.to_string(), true));
            }
            if let Some(path) = args.get("file_path").and_then(|v| v.as_str()) {
                paths.push((path.to_string(), true));
            }
        }
        "file_edit" | "edit" | "edit_file" => {
            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                paths.push((path.to_string(), true));
            }
            if let Some(path) = args.get("file_path").and_then(|v| v.as_str()) {
                paths.push((path.to_string(), true));
            }
        }
        "grep" | "search" | "glob" => {
            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                paths.push((path.to_string(), false));
            }
        }
        "bash" | "shell" => {
            // For bash commands, extract the command and do basic path
            // analysis. This is best-effort — a truly sandboxed shell
            // would need namespace isolation.
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                // Detect common file-modifying commands.
                let write_prefixes = ["rm ", "mv ", "cp ", "mkdir ", "touch ", "chmod "];
                let is_write = write_prefixes.iter().any(|p| cmd.starts_with(p));
                // Extract paths from the command (simple heuristic: tokens
                // starting with '/').
                for token in cmd.split_whitespace() {
                    if token.starts_with('/') {
                        paths.push((token.to_string(), is_write));
                    }
                }
            }
        }
        _ => {}
    }

    paths
}

#[async_trait]
impl Guard for PathGuard {
    async fn check_tool(&self, ctx: &GuardContext, tool_name: &str, args: &Value) -> Verdict {
        // Extract file paths from tool arguments and check each one.
        let file_paths = extract_file_paths(tool_name, args);
        for (path, is_write) in &file_paths {
            if let Err(e) = self.check_access(Path::new(path), *is_write) {
                return Verdict::Deny {
                    reason: e.to_string(),
                };
            }
        }

        // Delegate to inner guard for remaining checks.
        self.inner.check_tool(ctx, tool_name, args).await
    }

    async fn check_output(&self, ctx: &GuardContext, content: &str) -> Verdict {
        // PathGuard does not inspect output — delegate entirely.
        self.inner.check_output(ctx, content).await
    }
}

/// Canonicalize a path, falling back to lexical normalization if the path
/// does not exist on disk yet (e.g., the workspace hasn't been created).
fn canonicalize_or_normalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| normalize_path(path))
}

/// Resolve a path by canonicalizing its nearest existing ancestor, then
/// appending the remaining (non-existent) suffix.
///
/// This handles the common case where a file does not yet exist but its
/// parent directory does. On macOS, `/var` is a symlink to `/private/var`,
/// so `canonicalize()` on a non-existent file fails while its parent
/// resolves to `/private/var/...`. This function ensures consistent
/// resolution.
fn resolve_against_parents(path: &Path) -> PathBuf {
    // If the full path can be canonicalized, use that.
    if let Ok(c) = path.canonicalize() {
        return c;
    }

    // Walk up until we find an ancestor that exists.
    let normalized = normalize_path(path);
    let mut suffix = Vec::new();
    let mut current = normalized.as_path();
    loop {
        if let Ok(c) = current.canonicalize() {
            // Re-join the non-existent suffix.
            let mut result = c;
            for part in suffix.into_iter().rev() {
                result.push(part);
            }
            return result;
        }
        match current.file_name() {
            Some(name) => {
                suffix.push(name.to_os_string());
                current = match current.parent() {
                    Some(p) => p,
                    None => return normalized,
                };
            }
            None => return normalized,
        }
    }
}

/// Lexical normalization: resolve `.` and `..` without hitting the filesystem.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            other => components.push(other),
        }
    }
    components.iter().collect()
}

// ---------------------------------------------------------------------------
// Workspace lifecycle helpers
// ---------------------------------------------------------------------------

/// Create an isolated temporary workspace directory for an agent.
///
/// Returns the path to the created directory. The caller is responsible for
/// cleaning up the directory when the agent terminates.
pub fn create_workspace(agent_id: &crate::process::AgentId) -> Result<PathBuf, KernelError> {
    let dir = std::env::temp_dir().join(format!("rara-agent-{}", agent_id.0));
    std::fs::create_dir_all(&dir).map_err(|e| KernelError::SandboxPathError {
        message: format!("failed to create workspace: {e}"),
    })?;
    Ok(dir)
}

/// Clean up a workspace directory.
///
/// Silently ignores errors (best-effort cleanup).
pub fn cleanup_workspace(workspace: &Path) {
    if workspace.exists() {
        let _ = std::fs::remove_dir_all(workspace);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::defaults::noop::NoopGuard;

    /// Helper to build a PathGuard with a real temp workspace.
    fn make_guard(
        allowed: Vec<&str>,
        read_only: Vec<&str>,
        denied: Vec<&str>,
    ) -> (PathGuard, PathBuf) {
        let workspace =
            std::env::temp_dir().join(format!("path-guard-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&workspace).unwrap();

        let config = SandboxConfig {
            allowed_paths:      allowed.into_iter().map(String::from).collect(),
            read_only_paths:    read_only.into_iter().map(String::from).collect(),
            denied_paths:       denied.into_iter().map(String::from).collect(),
            isolated_workspace: true,
        };

        let guard = PathGuard::new(config, workspace.clone(), Box::new(NoopGuard));
        (guard, workspace)
    }

    #[test]
    fn test_workspace_always_allowed() {
        let (guard, workspace) = make_guard(vec![], vec![], vec![]);
        let file = workspace.join("test.txt");
        assert!(guard.check_access(&file, false).is_ok());
        assert!(guard.check_access(&file, true).is_ok());

        // Cleanup
        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn test_allowed_paths_read_write() {
        let allowed_dir = std::env::temp_dir().join(format!("allowed-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&allowed_dir).unwrap();

        let (guard, workspace) = make_guard(vec![allowed_dir.to_str().unwrap()], vec![], vec![]);

        let file = allowed_dir.join("data.txt");
        assert!(guard.check_access(&file, false).is_ok());
        assert!(guard.check_access(&file, true).is_ok());

        let _ = fs::remove_dir_all(&workspace);
        let _ = fs::remove_dir_all(&allowed_dir);
    }

    #[test]
    fn test_read_only_paths_block_writes() {
        let ro_dir = std::env::temp_dir().join(format!("readonly-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&ro_dir).unwrap();

        let (guard, workspace) = make_guard(vec![], vec![ro_dir.to_str().unwrap()], vec![]);

        let file = ro_dir.join("config.toml");
        assert!(
            guard.check_access(&file, false).is_ok(),
            "read should be allowed"
        );
        assert!(
            guard.check_access(&file, true).is_err(),
            "write should be denied"
        );

        let _ = fs::remove_dir_all(&workspace);
        let _ = fs::remove_dir_all(&ro_dir);
    }

    #[test]
    fn test_denied_paths_take_precedence() {
        let dir = std::env::temp_dir().join(format!("denied-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let secrets = dir.join("secrets");
        fs::create_dir_all(&secrets).unwrap();

        let (guard, workspace) = make_guard(
            vec![dir.to_str().unwrap()],
            vec![],
            vec![secrets.to_str().unwrap()],
        );

        // Parent dir is allowed.
        let normal_file = dir.join("normal.txt");
        assert!(guard.check_access(&normal_file, false).is_ok());
        assert!(guard.check_access(&normal_file, true).is_ok());

        // Secrets subdirectory is denied even though parent is allowed.
        let secret_file = secrets.join("key.pem");
        assert!(guard.check_access(&secret_file, false).is_err());
        assert!(guard.check_access(&secret_file, true).is_err());

        let _ = fs::remove_dir_all(&workspace);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_unlisted_path_denied() {
        let (guard, workspace) = make_guard(vec![], vec![], vec![]);
        let random_path = Path::new("/some/random/path/file.txt");
        assert!(guard.check_access(random_path, false).is_err());
        assert!(guard.check_access(random_path, true).is_err());

        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn test_resolve_within_workspace() {
        let (guard, workspace) = make_guard(vec![], vec![], vec![]);

        let resolved = guard.resolve("subdir/file.txt").unwrap();
        // Use canonicalized workspace for comparison (macOS has /var -> /private/var).
        let workspace_canonical = canonicalize_or_normalize(&workspace);
        assert!(resolved.starts_with(&workspace_canonical));

        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn test_resolve_traversal_blocked() {
        let (guard, workspace) = make_guard(vec![], vec![], vec![]);

        let result = guard.resolve("../../etc/passwd");
        assert!(result.is_err(), "path traversal should be blocked");

        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn test_resolve_absolute_path_outside_workspace() {
        let (guard, workspace) = make_guard(vec![], vec![], vec![]);

        let result = guard.resolve("/etc/passwd");
        assert!(
            result.is_err(),
            "absolute path outside workspace should fail"
        );

        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn test_extract_file_paths_read() {
        let args = serde_json::json!({"path": "/tmp/test.txt"});
        let paths = extract_file_paths("file_read", &args);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].0, "/tmp/test.txt");
        assert!(!paths[0].1, "should be read-only");
    }

    #[test]
    fn test_extract_file_paths_write() {
        let args = serde_json::json!({"file_path": "/tmp/output.txt"});
        let paths = extract_file_paths("file_write", &args);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].0, "/tmp/output.txt");
        assert!(paths[0].1, "should be write");
    }

    #[test]
    fn test_extract_file_paths_bash() {
        let args = serde_json::json!({"command": "rm /tmp/test.txt"});
        let paths = extract_file_paths("bash", &args);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].0, "/tmp/test.txt");
        assert!(paths[0].1, "rm should be detected as write");
    }

    #[test]
    fn test_extract_file_paths_bash_read() {
        let args = serde_json::json!({"command": "cat /tmp/test.txt"});
        let paths = extract_file_paths("bash", &args);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].0, "/tmp/test.txt");
        assert!(!paths[0].1, "cat should be detected as read");
    }

    #[test]
    fn test_extract_file_paths_unknown_tool() {
        let args = serde_json::json!({"path": "/tmp/test.txt"});
        let paths = extract_file_paths("unknown_tool", &args);
        assert!(paths.is_empty(), "unknown tools should return no paths");
    }

    #[tokio::test]
    async fn test_guard_trait_denies_forbidden_path() {
        let denied_dir = std::env::temp_dir().join(format!("guard-deny-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&denied_dir).unwrap();

        let (guard, workspace) = make_guard(vec![], vec![], vec![denied_dir.to_str().unwrap()]);

        let ctx = GuardContext {
            agent_id:   uuid::Uuid::nil(),
            user_id:    uuid::Uuid::nil(),
            session_id: uuid::Uuid::nil(),
        };

        let denied_file = denied_dir.join("secret.key");
        let args = serde_json::json!({"path": denied_file.to_str().unwrap()});
        let verdict = guard.check_tool(&ctx, "file_read", &args).await;
        assert!(verdict.is_deny(), "reading denied path should be rejected");

        let _ = fs::remove_dir_all(&workspace);
        let _ = fs::remove_dir_all(&denied_dir);
    }

    #[tokio::test]
    async fn test_guard_trait_allows_workspace_path() {
        let (guard, workspace) = make_guard(vec![], vec![], vec![]);

        let ctx = GuardContext {
            agent_id:   uuid::Uuid::nil(),
            user_id:    uuid::Uuid::nil(),
            session_id: uuid::Uuid::nil(),
        };

        let workspace_file = workspace.join("data.json");
        let args = serde_json::json!({"path": workspace_file.to_str().unwrap()});
        let verdict = guard.check_tool(&ctx, "file_write", &args).await;
        assert!(verdict.is_allow(), "writing to workspace should be allowed");

        let _ = fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn test_guard_trait_read_only_blocks_write() {
        let ro_dir = std::env::temp_dir().join(format!("guard-ro-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&ro_dir).unwrap();

        let (guard, workspace) = make_guard(vec![], vec![ro_dir.to_str().unwrap()], vec![]);

        let ctx = GuardContext {
            agent_id:   uuid::Uuid::nil(),
            user_id:    uuid::Uuid::nil(),
            session_id: uuid::Uuid::nil(),
        };

        let ro_file = ro_dir.join("config.yaml");

        // Read should be allowed.
        let args = serde_json::json!({"path": ro_file.to_str().unwrap()});
        let verdict = guard.check_tool(&ctx, "file_read", &args).await;
        assert!(
            verdict.is_allow(),
            "reading read-only path should be allowed"
        );

        // Write should be denied.
        let verdict = guard.check_tool(&ctx, "file_write", &args).await;
        assert!(verdict.is_deny(), "writing read-only path should be denied");

        let _ = fs::remove_dir_all(&workspace);
        let _ = fs::remove_dir_all(&ro_dir);
    }

    #[test]
    fn test_create_and_cleanup_workspace() {
        let agent_id = crate::process::AgentId::new();
        let workspace = create_workspace(&agent_id).unwrap();
        assert!(workspace.exists());
        assert!(workspace.is_dir());

        cleanup_workspace(&workspace);
        assert!(!workspace.exists());
    }

    #[test]
    fn test_normalize_path() {
        let normalized = normalize_path(Path::new("/a/b/../c/./d"));
        assert_eq!(normalized, PathBuf::from("/a/c/d"));
    }

    #[test]
    fn test_sandbox_config_default() {
        let config = SandboxConfig::default();
        assert!(config.allowed_paths.is_empty());
        assert!(config.read_only_paths.is_empty());
        assert!(config.denied_paths.is_empty());
        assert!(!config.isolated_workspace);
    }

    #[test]
    fn test_sandbox_config_yaml_roundtrip() {
        let config = SandboxConfig {
            allowed_paths:      vec!["/tmp/work".to_string()],
            read_only_paths:    vec!["/etc/config".to_string()],
            denied_paths:       vec!["/etc/secrets".to_string()],
            isolated_workspace: true,
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: SandboxConfig = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(deserialized.allowed_paths, config.allowed_paths);
        assert_eq!(deserialized.read_only_paths, config.read_only_paths);
        assert_eq!(deserialized.denied_paths, config.denied_paths);
        assert_eq!(deserialized.isolated_workspace, config.isolated_workspace);
    }

    #[test]
    fn test_agent_manifest_with_sandbox_yaml() {
        let yaml = r#"
name: sandboxed-agent
description: "Agent with sandbox"
model: "gpt-4"
system_prompt: "You are sandboxed."
sandbox:
  allowed_paths:
    - /tmp/shared
  read_only_paths:
    - /etc/config
  denied_paths:
    - /etc/secrets
  isolated_workspace: true
"#;
        let manifest: crate::process::AgentManifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(manifest.name, "sandboxed-agent");
        let sandbox = manifest.sandbox.unwrap();
        assert_eq!(sandbox.allowed_paths, vec!["/tmp/shared"]);
        assert_eq!(sandbox.read_only_paths, vec!["/etc/config"]);
        assert_eq!(sandbox.denied_paths, vec!["/etc/secrets"]);
        assert!(sandbox.isolated_workspace);
    }

    #[test]
    fn test_agent_manifest_without_sandbox_yaml() {
        let yaml = r#"
name: unsandboxed
description: "No sandbox"
model: "gpt-4"
system_prompt: "Hello"
"#;
        let manifest: crate::process::AgentManifest = serde_yaml::from_str(yaml).unwrap();
        assert!(manifest.sandbox.is_none());
    }

    #[test]
    fn test_update_config_at_runtime() {
        let (guard, workspace) = make_guard(vec![], vec![], vec![]);

        // Initially, an external path should be denied.
        let external_dir = std::env::temp_dir().join(format!("dynamic-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&external_dir).unwrap();
        let file = external_dir.join("data.txt");
        assert!(
            guard.check_access(&file, false).is_err(),
            "should be denied before update"
        );

        // Update config to allow the external directory.
        guard.update_config(SandboxConfig {
            allowed_paths:      vec![external_dir.to_str().unwrap().to_string()],
            read_only_paths:    vec![],
            denied_paths:       vec![],
            isolated_workspace: true,
        });

        // Now it should be allowed.
        assert!(
            guard.check_access(&file, false).is_ok(),
            "should be allowed after update"
        );
        assert!(
            guard.check_access(&file, true).is_ok(),
            "write should be allowed after update"
        );

        let _ = fs::remove_dir_all(&workspace);
        let _ = fs::remove_dir_all(&external_dir);
    }
}
