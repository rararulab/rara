# Approval Manager Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an `ApprovalManager` to the kernel that gates dangerous tool executions behind human approval, with in-memory pending request queue and oneshot channel blocking.

**Architecture:** The `ApprovalManager` lives in the kernel crate as a standalone module (`approval.rs`). It uses `DashMap` for thread-safe pending request storage and `tokio::sync::oneshot` channels to block agent execution until a human resolves (approve/deny/timeout). The manager is injected into `KernelInner` as an `Arc<ApprovalManager>` and wired into the existing `Syscall::RequiresApproval` / `Syscall::RequestApproval` handlers in `event_loop.rs`. No new syscall variants are needed.

**Tech Stack:** Rust, tokio (oneshot + timeout), dashmap, uuid, jiff, serde, tracing

**Reference:** [openfang approval.rs](https://github.com/RightNow-AI/openfang/blob/main/crates/openfang-kernel/src/approval.rs)

---

## Task 1: Define Approval Types

**Files:**
- Create: `crates/core/kernel/src/approval.rs`
- Modify: `crates/core/kernel/src/lib.rs` (add `pub mod approval;`)

### Step 1: Create the approval module with types

Create `crates/core/kernel/src/approval.rs` with the following types:

```rust
//! Execution approval manager — gates dangerous operations behind human approval.

use std::sync::RwLock;

use dashmap::DashMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::process::AgentId;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Risk level classification for tool invocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Decision outcome for an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Denied,
    TimedOut,
}

/// An approval request submitted by an agent before executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id:             Uuid,
    pub agent_id:       AgentId,
    pub tool_name:      String,
    pub tool_args:      serde_json::Value,
    pub summary:        String,
    pub risk_level:     RiskLevel,
    pub requested_at:   Timestamp,
    pub timeout_secs:   u64,
}

/// Response after an approval request is resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResponse {
    pub request_id:  Uuid,
    pub decision:    ApprovalDecision,
    pub decided_at:  Timestamp,
    pub decided_by:  Option<String>,
}

/// Policy controlling which tools require approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    /// Tool names that always require human approval.
    pub require_approval: Vec<String>,
    /// Default timeout in seconds for approval requests.
    pub timeout_secs: u64,
    /// If true, auto-approve all requests (bypass mode).
    pub auto_approve: bool,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            require_approval: vec!["bash".to_string(), "shell_exec".to_string()],
            timeout_secs: 120,
            auto_approve: false,
        }
    }
}
```

### Step 2: Register the module in lib.rs

In `crates/core/kernel/src/lib.rs`, add after `pub mod audit;` (line 31):

```rust
pub mod approval;
```

### Step 3: Verify compilation

Run: `cargo check -p rara-kernel`
Expected: compiles cleanly

### Step 4: Commit

```bash
git add crates/core/kernel/src/approval.rs crates/core/kernel/src/lib.rs
git commit -m "feat(kernel): add approval types — RiskLevel, ApprovalRequest, ApprovalPolicy (#ISSUE)"
```

---

## Task 2: Implement ApprovalManager Core

**Files:**
- Modify: `crates/core/kernel/src/approval.rs`

### Step 1: Write tests first

Append the following to `approval.rs`:

```rust
/// Maximum pending requests per agent to prevent resource exhaustion.
const MAX_PENDING_PER_AGENT: usize = 5;

/// Internal pending request holding the oneshot sender.
struct PendingRequest {
    request: ApprovalRequest,
    sender:  tokio::sync::oneshot::Sender<ApprovalDecision>,
}

/// Manages approval requests with oneshot channels for blocking resolution.
///
/// When an agent calls a tool that requires approval, the agent's execution
/// blocks on a oneshot channel until a human resolves it (via `resolve()`)
/// or the request times out.
pub struct ApprovalManager {
    pending: DashMap<Uuid, PendingRequest>,
    policy:  RwLock<ApprovalPolicy>,
}

impl ApprovalManager {
    /// Create a new manager with the given policy.
    pub fn new(policy: ApprovalPolicy) -> Self {
        Self {
            pending: DashMap::new(),
            policy:  RwLock::new(policy),
        }
    }

    /// Check if a tool requires approval based on current policy.
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());
        if policy.auto_approve {
            return false;
        }
        policy.require_approval.iter().any(|t| t == tool_name)
    }

    /// Submit an approval request. Blocks until resolved or timed out.
    ///
    /// If `auto_approve` is enabled in the policy, returns `Approved` immediately.
    /// If the agent already has `MAX_PENDING_PER_AGENT` pending requests, returns `Denied`.
    pub async fn request_approval(&self, req: ApprovalRequest) -> ApprovalDecision {
        // Auto-approve bypass
        {
            let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());
            if policy.auto_approve {
                info!(request_id = %req.id, tool = %req.tool_name, "auto-approved");
                return ApprovalDecision::Approved;
            }
        }

        // Per-agent pending limit
        let agent_pending = self
            .pending
            .iter()
            .filter(|r| r.value().request.agent_id == req.agent_id)
            .count();
        if agent_pending >= MAX_PENDING_PER_AGENT {
            warn!(agent_id = ?req.agent_id, "approval rejected: too many pending");
            return ApprovalDecision::Denied;
        }

        let timeout = std::time::Duration::from_secs(req.timeout_secs);
        let id = req.id;

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.insert(id, PendingRequest { request: req, sender: tx });

        info!(request_id = %id, "approval request submitted, waiting for resolution");

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(decision)) => {
                debug!(request_id = %id, ?decision, "approval resolved");
                decision
            }
            _ => {
                self.pending.remove(&id);
                warn!(request_id = %id, "approval request timed out");
                ApprovalDecision::TimedOut
            }
        }
    }

    /// Resolve a pending request (called by external API / TG callback / WebSocket).
    pub fn resolve(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
        decided_by: Option<String>,
    ) -> Result<ApprovalResponse, String> {
        match self.pending.remove(&request_id) {
            Some((_, pending)) => {
                let response = ApprovalResponse {
                    request_id,
                    decision,
                    decided_at: Timestamp::now(),
                    decided_by,
                };
                let _ = pending.sender.send(decision);
                info!(request_id = %request_id, ?decision, "approval resolved");
                Ok(response)
            }
            None => Err(format!("no pending approval request: {request_id}")),
        }
    }

    /// List all pending requests (for dashboard / API).
    pub fn list_pending(&self) -> Vec<ApprovalRequest> {
        self.pending.iter().map(|r| r.value().request.clone()).collect()
    }

    /// Number of pending requests.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Update the approval policy (hot-reload).
    pub fn update_policy(&self, policy: ApprovalPolicy) {
        *self.policy.write().unwrap_or_else(|e| e.into_inner()) = policy;
    }

    /// Get a copy of the current policy.
    pub fn policy(&self) -> ApprovalPolicy {
        self.policy.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Classify the risk level of a tool invocation.
    pub fn classify_risk(tool_name: &str) -> RiskLevel {
        match tool_name {
            "bash" | "shell_exec" => RiskLevel::Critical,
            "file_write" | "file_delete" | "write" | "edit" => RiskLevel::High,
            "web_fetch" | "browser_navigate" => RiskLevel::Medium,
            _ => RiskLevel::Low,
        }
    }
}
```

### Step 2: Write unit tests

Append tests to `approval.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use super::*;

    fn default_manager() -> ApprovalManager {
        ApprovalManager::new(ApprovalPolicy::default())
    }

    fn make_request(agent_name: &str, tool_name: &str, timeout_secs: u64) -> ApprovalRequest {
        ApprovalRequest {
            id:           Uuid::new_v4(),
            agent_id:     AgentId::new(),
            tool_name:    tool_name.to_string(),
            tool_args:    serde_json::json!({}),
            summary:      format!("execute {tool_name}"),
            risk_level:   ApprovalManager::classify_risk(tool_name),
            requested_at: Timestamp::now(),
            timeout_secs,
        }
    }

    #[test]
    fn requires_approval_default_policy() {
        let mgr = default_manager();
        assert!(mgr.requires_approval("bash"));
        assert!(mgr.requires_approval("shell_exec"));
        assert!(!mgr.requires_approval("file_read"));
    }

    #[test]
    fn requires_approval_custom_policy() {
        let policy = ApprovalPolicy {
            require_approval: vec!["file_write".to_string()],
            timeout_secs: 30,
            auto_approve: false,
        };
        let mgr = ApprovalManager::new(policy);
        assert!(mgr.requires_approval("file_write"));
        assert!(!mgr.requires_approval("bash"));
    }

    #[test]
    fn requires_approval_auto_approve_bypasses() {
        let policy = ApprovalPolicy {
            require_approval: vec!["bash".to_string()],
            timeout_secs: 60,
            auto_approve: true,
        };
        let mgr = ApprovalManager::new(policy);
        assert!(!mgr.requires_approval("bash"));
    }

    #[test]
    fn classify_risk_levels() {
        assert_eq!(ApprovalManager::classify_risk("bash"), RiskLevel::Critical);
        assert_eq!(ApprovalManager::classify_risk("shell_exec"), RiskLevel::Critical);
        assert_eq!(ApprovalManager::classify_risk("file_write"), RiskLevel::High);
        assert_eq!(ApprovalManager::classify_risk("file_delete"), RiskLevel::High);
        assert_eq!(ApprovalManager::classify_risk("web_fetch"), RiskLevel::Medium);
        assert_eq!(ApprovalManager::classify_risk("file_read"), RiskLevel::Low);
        assert_eq!(ApprovalManager::classify_risk("unknown"), RiskLevel::Low);
    }

    #[test]
    fn resolve_nonexistent_returns_error() {
        let mgr = default_manager();
        let result = mgr.resolve(Uuid::new_v4(), ApprovalDecision::Approved, None);
        assert!(result.is_err());
    }

    #[test]
    fn list_pending_empty() {
        let mgr = default_manager();
        assert!(mgr.list_pending().is_empty());
        assert_eq!(mgr.pending_count(), 0);
    }

    #[test]
    fn update_policy_hot_reload() {
        let mgr = default_manager();
        assert!(mgr.requires_approval("bash"));

        mgr.update_policy(ApprovalPolicy {
            require_approval: vec!["file_write".to_string()],
            timeout_secs: 30,
            auto_approve: false,
        });

        assert!(!mgr.requires_approval("bash"));
        assert!(mgr.requires_approval("file_write"));
        assert_eq!(mgr.policy().timeout_secs, 30);
    }

    #[tokio::test]
    async fn request_approval_auto_approve() {
        let policy = ApprovalPolicy {
            require_approval: vec!["bash".to_string()],
            timeout_secs: 60,
            auto_approve: true,
        };
        let mgr = ApprovalManager::new(policy);
        let req = make_request("agent-1", "bash", 60);
        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Approved);
        assert_eq!(mgr.pending_count(), 0);
    }

    #[tokio::test]
    async fn request_approval_timeout() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "bash", 1); // 1 second timeout
        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::TimedOut);
        assert_eq!(mgr.pending_count(), 0);
    }

    #[tokio::test]
    async fn request_approval_approved() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "bash", 60);
        let request_id = req.id;

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let result = mgr2.resolve(request_id, ApprovalDecision::Approved, Some("admin".into()));
            assert!(result.is_ok());
        });

        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Approved);
    }

    #[tokio::test]
    async fn request_approval_denied() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "bash", 60);
        let request_id = req.id;

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let _ = mgr2.resolve(request_id, ApprovalDecision::Denied, None);
        });

        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Denied);
    }

    #[tokio::test]
    async fn max_pending_per_agent() {
        let mgr = Arc::new(default_manager());
        let agent_id = AgentId::new();

        // Fill up MAX_PENDING_PER_AGENT requests
        let mut ids = Vec::new();
        for _ in 0..MAX_PENDING_PER_AGENT {
            let mut req = make_request("agent-1", "bash", 300);
            req.agent_id = agent_id;
            ids.push(req.id);
            let mgr_clone = Arc::clone(&mgr);
            tokio::spawn(async move {
                mgr_clone.request_approval(req).await;
            });
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(mgr.pending_count(), MAX_PENDING_PER_AGENT);

        // Next request from same agent should be denied
        let mut req6 = make_request("agent-1", "bash", 300);
        req6.agent_id = agent_id;
        let decision = mgr.request_approval(req6).await;
        assert_eq!(decision, ApprovalDecision::Denied);

        // Different agent can still submit
        let req_other = make_request("agent-2", "bash", 300);
        let other_id = req_other.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move { mgr2.request_approval(req_other).await; });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(mgr.pending_count(), MAX_PENDING_PER_AGENT + 1);

        // Cleanup
        for id in &ids {
            let _ = mgr.resolve(*id, ApprovalDecision::Denied, None);
        }
        let _ = mgr.resolve(other_id, ApprovalDecision::Denied, None);
    }
}
```

### Step 3: Run tests

Run: `cargo test -p rara-kernel approval`
Expected: all tests pass

### Step 4: Commit

```bash
git add crates/core/kernel/src/approval.rs
git commit -m "feat(kernel): implement ApprovalManager with policy, risk classification, and blocking resolution (#ISSUE)"
```

---

## Task 3: Wire ApprovalManager into KernelInner

**Files:**
- Modify: `crates/core/kernel/src/kernel.rs` (add field + constructor param)
- Modify: `crates/core/kernel/src/event_loop.rs` (update syscall handlers)

### Step 1: Add `approval` field to `KernelInner`

In `crates/core/kernel/src/kernel.rs`, add to the `KernelInner` struct (after `audit_log` field, around line 119):

```rust
/// Approval manager for gating dangerous tool executions.
pub approval: Arc<crate::approval::ApprovalManager>,
```

### Step 2: Add `approval` parameter to `Kernel::new()`

In the `Kernel::new()` signature (line 263), add a new parameter after `audit_log`:

```rust
approval: Arc<crate::approval::ApprovalManager>,
```

And in the `KernelInner` construction block (around line 295), add:

```rust
approval,
```

### Step 3: Update `handle_syscall()` in event_loop.rs

In `crates/core/kernel/src/event_loop.rs`, update the two approval-related syscall handlers:

**Replace `Syscall::RequiresApproval` handler** (lines 341-344):

```rust
Syscall::RequiresApproval { tool_name, reply_tx } => {
    let result = inner.approval.requires_approval(&tool_name);
    let _ = reply_tx.send(result);
}
```

**Replace `Syscall::RequestApproval` handler** (lines 346-367):

```rust
Syscall::RequestApproval {
    agent_id,
    principal: _,
    tool_name,
    summary,
    reply_tx,
} => {
    let approval = Arc::clone(&inner.approval);
    let policy = approval.policy();
    let req = crate::approval::ApprovalRequest {
        id:           uuid::Uuid::new_v4(),
        agent_id,
        tool_name:    tool_name.clone(),
        tool_args:    serde_json::json!({"summary": &summary}),
        summary,
        risk_level:   crate::approval::ApprovalManager::classify_risk(&tool_name),
        requested_at: Timestamp::now(),
        timeout_secs: policy.timeout_secs,
    };

    // Spawn a task so the event loop is not blocked while waiting
    // for human approval.
    tokio::spawn(async move {
        let decision = approval.request_approval(req).await;
        let approved = matches!(decision, crate::approval::ApprovalDecision::Approved);
        let _ = reply_tx.send(Ok(approved));
    });
}
```

### Step 4: Fix all callers of `Kernel::new()`

Search for all call sites of `Kernel::new()` and add the `approval` parameter. The main call sites are:

1. **`crates/core/boot/src/kernel.rs`** — the `boot()` function. Add:
   - A new `approval` field to `BootConfig`
   - Default to `Arc::new(ApprovalManager::new(ApprovalPolicy::default()))`
   - Pass to `Kernel::new()`

2. **`crates/core/kernel/src/testing.rs`** — the test harness. Add default `ApprovalManager`.

3. **Any other test files** that call `Kernel::new()` directly.

For boot, in `crates/core/boot/src/kernel.rs`:

Add import:
```rust
use rara_kernel::approval::{ApprovalManager, ApprovalPolicy};
```

Add to `BootConfig`:
```rust
/// Approval manager (optional — defaults to ApprovalManager with default policy).
pub approval: Option<Arc<ApprovalManager>>,
```

In `BootConfig::default()`, add:
```rust
approval: None,
```

In `boot()`, add before `Kernel::new()`:
```rust
let approval = config
    .approval
    .unwrap_or_else(|| Arc::new(ApprovalManager::new(ApprovalPolicy::default())));
```

Pass `approval` to `Kernel::new()`.

### Step 5: Verify compilation

Run: `cargo check -p rara-kernel && cargo check -p rara-boot`
Expected: compiles cleanly

### Step 6: Run all kernel tests

Run: `cargo test -p rara-kernel`
Expected: all existing tests pass (236+)

### Step 7: Commit

```bash
git add crates/core/kernel/src/kernel.rs crates/core/kernel/src/event_loop.rs \
        crates/core/boot/src/kernel.rs crates/core/kernel/src/testing.rs
git commit -m "feat(kernel): wire ApprovalManager into KernelInner and event loop (#ISSUE)"
```

---

## Task 4: Expose ApprovalManager on Kernel Public API

**Files:**
- Modify: `crates/core/kernel/src/kernel.rs` (add accessor method)

### Step 1: Add `approval()` accessor to `Kernel`

In the `Kernel` impl block (around line 256), add a public accessor:

```rust
/// Get a reference to the approval manager.
pub fn approval(&self) -> &Arc<crate::approval::ApprovalManager> {
    &self.inner.approval
}
```

### Step 2: Re-export key types from lib.rs

In `crates/core/kernel/src/lib.rs`, add to the existing `pub use` block:

```rust
pub use approval::{ApprovalDecision, ApprovalManager, ApprovalPolicy, ApprovalRequest, ApprovalResponse, RiskLevel};
```

### Step 3: Verify compilation

Run: `cargo check -p rara-kernel`
Expected: compiles cleanly

### Step 4: Commit

```bash
git add crates/core/kernel/src/kernel.rs crates/core/kernel/src/lib.rs
git commit -m "feat(kernel): expose ApprovalManager on Kernel public API (#ISSUE)"
```

---

## Task 5: Integration Test — Full Approval Flow

**Files:**
- Modify: `crates/core/kernel/src/approval.rs` (add integration-style tests)

### Step 1: Add integration test using the testing harness

If the kernel has a `testing` module with a `TestKernel` builder, use it. Otherwise add a test in `approval.rs` that verifies the full flow through ProcessHandle syscalls. At minimum, verify the `ApprovalManager` works correctly standalone (the unit tests from Task 2 already cover this).

Add one more test to the existing test module in `approval.rs`:

```rust
#[tokio::test]
async fn list_pending_shows_active_requests() {
    let mgr = Arc::new(default_manager());
    let req = make_request("agent-1", "bash", 300);
    let request_id = req.id;
    let tool = req.tool_name.clone();

    let mgr2 = Arc::clone(&mgr);
    tokio::spawn(async move { mgr2.request_approval(req).await; });

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let pending = mgr.list_pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, request_id);
    assert_eq!(pending[0].tool_name, tool);

    // Cleanup
    let _ = mgr.resolve(request_id, ApprovalDecision::Denied, None);
}
```

### Step 2: Run full test suite

Run: `cargo test -p rara-kernel`
Expected: all tests pass

### Step 3: Final commit

```bash
git add crates/core/kernel/src/approval.rs
git commit -m "test(kernel): add integration test for approval pending list (#ISSUE)"
```

---

## Summary

| Task | What | Files |
|------|------|-------|
| 1 | Approval types (RiskLevel, ApprovalRequest, ApprovalPolicy, etc.) | `approval.rs`, `lib.rs` |
| 2 | ApprovalManager implementation + unit tests | `approval.rs` |
| 3 | Wire into KernelInner + event_loop syscall handlers + boot | `kernel.rs`, `event_loop.rs`, boot `kernel.rs`, `testing.rs` |
| 4 | Public API accessor + re-exports | `kernel.rs`, `lib.rs` |
| 5 | Integration tests | `approval.rs` |

After this is complete, the ApprovalManager will be functional in the kernel. Future issues can add:
- HTTP REST endpoints for approval management
- Telegram inline keyboard integration (callback handler)
- WebSocket push for real-time approval notifications
