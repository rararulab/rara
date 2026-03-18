# Typed Tool Result Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `type Output: Serialize` to `ToolExecute` trait so tools return typed structs instead of raw `json!`.

**Architecture:** Change `ToolExecute::run` return type from `ToolOutput` to `Self::Output`. Macro auto-serializes via `ToolOutput::from_serialize()`. Old tools use `type Output = serde_json::Value` for compat. Demo migrate bash + read_file.

**Tech Stack:** Rust, serde, async_trait, proc_macro (tool-macro crate)

---

### Task 1: Add `Output` associated type to `ToolExecute` and `from_serialize` to `ToolOutput`

**Files:**
- Modify: `crates/kernel/src/tool/mod.rs:34-42` (ToolExecute trait)
- Modify: `crates/kernel/src/tool/mod.rs:57-75` (ToolOutput impl)

**Step 1: Modify `ToolExecute` trait**

In `crates/kernel/src/tool/mod.rs`, change:

```rust
#[async_trait]
pub trait ToolExecute: Send + Sync {
    type Params: DeserializeOwned + schemars::JsonSchema;
    type Output: serde::Serialize;

    async fn run(&self, params: Self::Params, context: &ToolContext) -> anyhow::Result<Self::Output>;
}
```

Add `use serde::Serialize;` to imports if not already present.

**Step 2: Add `from_serialize` to `ToolOutput`**

Add this impl block after the existing `From<serde_json::Value>` impl:

```rust
impl ToolOutput {
    /// Create a `ToolOutput` by serializing a typed result struct.
    pub fn from_serialize<T: Serialize>(val: &T) -> anyhow::Result<Self> {
        Ok(Self {
            json: serde_json::to_value(val)?,
            resources: vec![],
        })
    }
}
```

**Step 3: Verify it compiles (expect errors from existing ToolExecute impls)**

Run: `cargo check -p rara-kernel 2>&1 | head -30`
Expected: Compile errors in schedule.rs, cancel_background.rs, create_plan.rs — they lack `type Output`.

**Step 4: Commit**

```bash
git add crates/kernel/src/tool/mod.rs
git commit -m "refactor(tool): add Output associated type to ToolExecute trait (#524)"
```

---

### Task 2: Update existing `ToolExecute` impls to add `type Output = serde_json::Value`

These tools currently return `ToolOutput` with `json!()`. For backward compat, set `type Output = serde_json::Value` and change return type from `anyhow::Result<ToolOutput>` to `anyhow::Result<serde_json::Value>`. The macro will handle wrapping.

But wait — these tools use the default macro path (no `execute_fn`), so the macro calls `ToolExecute::run` and needs to wrap the result. We need to update the macro first (Task 3) before these will compile. So for now, just add `type Output` and adjust return types.

**Files:**
- Modify: `crates/kernel/src/tool/create_plan.rs`
- Modify: `crates/kernel/src/tool/cancel_background.rs`
- Modify: `crates/kernel/src/tool/schedule.rs` (5 impls)

**Step 1: Update create_plan.rs**

Change the impl to:
```rust
#[async_trait]
impl super::ToolExecute for CreatePlanTool {
    type Params = CreatePlanParams;
    type Output = serde_json::Value;

    async fn run(
        &self,
        input: CreatePlanParams,
        _context: &super::ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        // ... existing body unchanged, already returns json via serde_json::to_value ...
        let json = serde_json::to_value(&plan)
            .map_err(|e| anyhow::anyhow!("failed to serialize plan: {e}"))?;
        Ok(json)  // was: Ok(json.into())
    }
}
```

**Step 2: Update cancel_background.rs**

Add `type Output = serde_json::Value;` and change return type. Remove `.into()` calls since we now return `Value` directly:

```rust
type Output = serde_json::Value;

async fn run(...) -> anyhow::Result<serde_json::Value> {
    // Change all `Ok(serde_json::json!({...}).into())` to `Ok(serde_json::json!({...}))`
}
```

**Step 3: Update schedule.rs**

Same pattern for all 5 impls: add `type Output = serde_json::Value;`, change return type, remove `.into()`.

**Step 4: Verify kernel compiles (still expect macro errors)**

Run: `cargo check -p rara-kernel 2>&1 | head -30`
Expected: May still fail until macro is updated in Task 3.

**Step 5: Commit**

```bash
git add crates/kernel/src/tool/create_plan.rs crates/kernel/src/tool/cancel_background.rs crates/kernel/src/tool/schedule.rs
git commit -m "refactor(tool): add type Output = Value to existing ToolExecute impls (#524)"
```

---

### Task 3: Update `ToolDef` macro to serialize `Output`

**Files:**
- Modify: `crates/common/tool-macro/src/lib.rs:148-159` (execute body generation)

**Step 1: Change the default execute body**

The current default body (when no `execute_fn` is set):

```rust
quote! {
    let typed: <Self as crate::tool::ToolExecute>::Params =
        serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid params for '{}': {e}", self.name()))?;
    crate::tool::ToolExecute::run(self, typed, context).await
}
```

Change to:

```rust
quote! {
    let typed: <Self as crate::tool::ToolExecute>::Params =
        serde_json::from_value(params)
            .map_err(|e| anyhow::anyhow!("invalid params for '{}': {e}", self.name()))?;
    let output = crate::tool::ToolExecute::run(self, typed, context).await?;
    crate::tool::ToolOutput::from_serialize(&output)
}
```

The `execute()` method signature stays `-> anyhow::Result<ToolOutput>`, so `from_serialize` returning `anyhow::Result<ToolOutput>` fits perfectly.

**Step 2: Verify kernel compiles**

Run: `cargo check -p rara-kernel`
Expected: PASS — all existing `ToolExecute` impls have `type Output = serde_json::Value`, and `Value` implements `Serialize`.

**Step 3: Commit**

```bash
git add crates/common/tool-macro/src/lib.rs
git commit -m "refactor(tool-macro): serialize ToolExecute::Output in generated execute body (#524)"
```

---

### Task 4: Demo migrate `bash.rs` — typed params + typed result

**Files:**
- Modify: `crates/app/src/tools/bash.rs`

**Step 1: Define typed params and result structs, implement `ToolExecute`**

Replace the current `execute_fn = "self.exec"` approach with `ToolExecute` impl:

```rust
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute, ToolOutput};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BashParams {
    /// The shell command to execute
    command: String,
    /// Timeout in seconds (default 120)
    timeout: Option<u64>,
    /// Working directory for the command
    cwd: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BashResult {
    /// Process exit code (-1 if failed to execute or timed out)
    pub exit_code: i32,
    /// Combined stdout and stderr output
    pub stdout: String,
    /// Whether the command was killed due to timeout
    pub timed_out: bool,
    /// Whether the output was truncated
    pub truncated: bool,
}

#[derive(ToolDef)]
#[tool(
    name = "bash",
    description = "Execute a shell command via /bin/bash -c. Returns exit code, combined \
                   stdout/stderr, and whether the command timed out. Output is truncated to 50KB \
                   / 2000 lines."
)]
pub struct BashTool;
```

Remove `params_schema = "Self::schema()"` and `execute_fn = "self.exec"` from the `#[tool(...)]` attribute. Remove the `schema()` and `exec()` methods. Replace with:

```rust
#[async_trait]
impl ToolExecute for BashTool {
    type Params = BashParams;
    type Output = BashResult;

    async fn run(&self, params: BashParams, _context: &ToolContext) -> anyhow::Result<BashResult> {
        let timeout_secs = params.timeout.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let effective_command = rtk_rewrite(&params.command).await;

        let mut cmd = tokio::process::Command::new("/bin/bash");
        cmd.arg("-c").arg(&effective_command);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if let Some(dir) = &params.cwd {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(rara_paths::workspace_dir());
        }

        let timeout_dur = std::time::Duration::from_secs(timeout_secs);

        match tokio::time::timeout(timeout_dur, cmd.output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}{stderr}");
                let (truncated_output, was_truncated) = truncate_output(&combined);
                Ok(BashResult {
                    exit_code: output.status.code().unwrap_or(-1),
                    stdout: truncated_output,
                    timed_out: false,
                    truncated: was_truncated,
                })
            }
            Ok(Err(e)) => Ok(BashResult {
                exit_code: -1,
                stdout: format!("failed to execute command: {e}"),
                timed_out: false,
                truncated: false,
            }),
            Err(_) => Ok(BashResult {
                exit_code: -1,
                stdout: format!("command timed out after {timeout_secs}s"),
                timed_out: true,
                truncated: false,
            }),
        }
    }
}
```

Keep `rtk_rewrite()` and `truncate_output()` helper functions unchanged.

**Step 2: Verify it compiles**

Run: `cargo check -p rara-app`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/app/src/tools/bash.rs
git commit -m "refactor(tool): migrate bash tool to typed params and result (#524)"
```

---

### Task 5: Demo migrate `read_file.rs` — typed params + typed result

**Files:**
- Modify: `crates/app/src/tools/read_file.rs`

**Step 1: Define typed params and result, implement `ToolExecute`**

Add params and result structs:

```rust
use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute, ToolOutput};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileParams {
    /// Absolute path to the file to read
    file_path: String,
    /// 1-based line number to start reading from (default 1)
    offset: Option<u64>,
    /// Maximum number of lines to return (default 2000)
    limit: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ReadFileResult {
    /// File content with line number prefixes
    pub content: String,
    /// Total number of lines in the file
    pub total_lines: usize,
    /// Whether the output was truncated
    pub truncated: bool,
}
```

Remove `params_schema` and `execute_fn` from `#[tool(...)]`. Remove `schema()` and `exec()` methods. Replace with `ToolExecute` impl:

```rust
#[async_trait]
impl ToolExecute for ReadFileTool {
    type Params = ReadFileParams;
    type Output = ReadFileResult;

    async fn run(&self, params: ReadFileParams, context: &ToolContext) -> anyhow::Result<ReadFileResult> {
        let raw_path = &params.file_path;
        let file_path = if std::path::Path::new(raw_path).is_absolute() {
            std::path::PathBuf::from(raw_path)
        } else {
            rara_paths::workspace_dir().join(raw_path)
        };

        let raw_bytes = tokio::fs::read(&file_path)
            .await
            .context(format!("failed to read file {}", file_path.display()))?;

        let check_len = raw_bytes.len().min(BINARY_CHECK_BYTES);
        if raw_bytes[..check_len].contains(&0) {
            return Ok(ReadFileResult {
                content: "[binary file detected]".to_owned(),
                total_lines: 0,
                truncated: false,
            });
        }

        let content = String::from_utf8_lossy(&raw_bytes);
        let all_lines: Vec<&str> = content.lines().collect();

        // Single-page mode
        if params.offset.is_some() || params.limit.is_some() {
            let offset = params.offset.map(|v| v.max(1) as usize).unwrap_or(1);
            let limit = params.limit.map(|v| v as usize).unwrap_or(DEFAULT_LIMIT);
            let page = read_page(&all_lines, offset, limit);
            return Ok(ReadFileResult {
                content: page.output,
                total_lines: page.total_lines,
                truncated: page.has_more_lines || page.content_truncated,
            });
        }

        // Adaptive paging mode
        let budget = compute_budget(context.context_window_tokens);
        let mut accumulated = String::new();
        let mut page_offset: usize = 1;
        let mut file_fully_read = false;
        let mut any_content_truncated = false;
        let mut total_lines = 0;

        for _ in 0..MAX_PAGES {
            let page = read_page(&all_lines, page_offset, DEFAULT_LIMIT);
            total_lines = page.total_lines;
            any_content_truncated |= page.content_truncated;
            accumulated.push_str(&page.output);

            if !page.has_more_lines {
                file_fully_read = true;
                break;
            }
            if accumulated.len() >= budget {
                break;
            }
            page_offset += page.lines_read;
        }

        if !file_fully_read {
            let last_line_no = accumulated
                .lines()
                .last()
                .and_then(|l| l.trim_start().split('\t').next())
                .and_then(|n| n.trim().parse::<usize>().ok())
                .unwrap_or(0);
            accumulated.push_str(&format!(
                "\n[Showing lines 1-{last_line_no} of {total_lines}. Use offset={next} to continue.]\n",
                next = last_line_no + 1,
            ));
        }

        Ok(ReadFileResult {
            content: accumulated,
            total_lines,
            truncated: !file_fully_read || any_content_truncated,
        })
    }
}
```

**Step 2: Verify it compiles**

Run: `cargo check -p rara-app`
Expected: PASS

**Step 3: Run existing tests**

Run: `cargo test -p rara-app -- read_file`
Expected: All existing tests pass (they test `read_page` and `compute_budget` which are unchanged).

**Step 4: Commit**

```bash
git add crates/app/src/tools/read_file.rs
git commit -m "refactor(tool): migrate read_file tool to typed params and result (#524)"
```

---

### Task 6: Full workspace check and final commit

**Step 1: Run full workspace check**

Run: `cargo check --workspace --all-targets`
Expected: PASS

**Step 2: Run pre-commit checks**

Run: `just pre-commit`
Expected: PASS (fmt + clippy + check + test)

**Step 3: Push and create PR**

```bash
git push -u origin issue-524-typed-tool-output
gh pr create --title "refactor(tool): typed Output associated type for ToolExecute (#524)" \
  --body "Closes #524" --label "refactor" --label "core"
```
