# Skills System Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an OpenClaw-inspired skills system to Rara — file-based skill definitions (Markdown + frontmatter) that control tool visibility, inject context into system prompts, and support auto-matching + manual activation.

**Architecture:** New `crates/skills/` crate (Layer 1) handles skill parsing/discovery/registry. `ChatService` integrates skills into the agent loop (prompt injection + tool filtering). Agent tools + HTTP API enable runtime skill management. Frontend provides UI for browsing/editing skills.

**Tech Stack:** Rust, serde + serde_yaml (frontmatter), regex (trigger matching), notify (fs watcher), axum (HTTP routes), React + TypeScript (frontend)

**Reference:** OpenClaw's SKILL.md format — each skill is a `.md` file with YAML frontmatter containing `name`, `description`, `tools` (whitelist), `trigger` (regex/keywords), and markdown body as the skill's prompt injection.

---

## Skill File Format

```
~/.config/job/skills/       # skills_dir() — user skills
skills/                      # project-root bundled skills (fallback)
```

Each skill is a single `.md` file:

```markdown
---
name: job-search
description: 职位搜索专家，帮你找到理想工作
tools:
  - job_pipeline
  - memory_search
  - http_fetch
trigger: "找工作|job search|搜索职位|推荐岗位"
enabled: true
---

你是一个职位搜索专家。当用户描述求职需求时：

1. 使用 memory_search 了解用户背景
2. 使用 job_pipeline 搜索匹配岗位
3. 返回结构化的推荐列表
```

### Frontmatter fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | yes | — | Unique identifier (also used as slug) |
| `description` | string | yes | — | Brief explanation shown in skill listing |
| `tools` | string[] | no | `[]` (all tools) | Whitelist of tool names visible to LLM |
| `trigger` | string | no | none | Regex pattern for auto-matching user messages |
| `enabled` | bool | no | `true` | Whether this skill is loaded |

### Loading precedence

1. `skills_dir()` (`~/.config/job/skills/`) — user skills (highest)
2. Project-root `skills/` — bundled skills (lowest)

When names conflict, user skills win.

---

## Task 1: Core skill types + loader crate (`crates/skills/`)

**Files:**
- Create: `crates/skills/Cargo.toml`
- Create: `crates/skills/src/lib.rs`
- Create: `crates/skills/src/types.rs`
- Create: `crates/skills/src/loader.rs`
- Create: `crates/skills/src/registry.rs`
- Create: `crates/skills/src/error.rs`
- Modify: `Cargo.toml` (workspace members)
- Modify: `crates/paths/src/lib.rs` (add `skills_dir()` + `bundled_skills_dir()`)

### types.rs

```rust
use std::path::PathBuf;
use serde::Deserialize;

/// YAML frontmatter parsed from a SKILL.md file.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tools: Vec<String>,
    pub trigger: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool { true }

/// A fully loaded skill: metadata + prompt body + source path.
#[derive(Debug, Clone)]
pub struct Skill {
    pub metadata: SkillMetadata,
    /// The markdown body (everything after the frontmatter).
    pub prompt: String,
    /// Filesystem path this skill was loaded from.
    pub source_path: PathBuf,
}

impl Skill {
    pub fn name(&self) -> &str { &self.metadata.name }
    pub fn description(&self) -> &str { &self.metadata.description }
    pub fn tools(&self) -> &[String] { &self.metadata.tools }
    pub fn trigger_pattern(&self) -> Option<&str> { self.metadata.trigger.as_deref() }
    pub fn is_enabled(&self) -> bool { self.metadata.enabled }
}
```

### loader.rs

```rust
use std::path::Path;
use crate::{error::SkillError, types::{Skill, SkillMetadata}};

/// Parse a single SKILL.md file into a `Skill`.
pub fn parse_skill_file(path: &Path) -> Result<Skill, SkillError> {
    let content = std::fs::read_to_string(path)?;
    let (metadata, prompt) = split_frontmatter(&content)?;
    Ok(Skill { metadata, prompt, source_path: path.to_owned() })
}

/// Scan a directory for .md files and parse each as a skill.
pub fn discover_skills(dir: &Path) -> Vec<Result<Skill, SkillError>> {
    // read_dir, filter .md, parse each
}

/// Split "---\nyaml\n---\nbody" into (SkillMetadata, body_string).
fn split_frontmatter(content: &str) -> Result<(SkillMetadata, String), SkillError> {
    // Find opening "---", find closing "---", parse YAML between them
}
```

### registry.rs

```rust
use std::collections::HashMap;
use regex::Regex;
use crate::types::Skill;

pub struct SkillRegistry {
    skills: HashMap<String, Skill>,
    /// Compiled trigger patterns, keyed by skill name.
    triggers: HashMap<String, Regex>,
}

impl SkillRegistry {
    pub fn new() -> Self { ... }

    /// Load skills from multiple directories with precedence.
    pub fn load_from_dirs(dirs: &[&Path]) -> Result<Self, SkillError> {
        // Later dirs override earlier ones (user > bundled)
    }

    /// Find all skills whose trigger matches the given text.
    pub fn match_triggers(&self, text: &str) -> Vec<&Skill> { ... }

    /// Get a skill by name.
    pub fn get(&self, name: &str) -> Option<&Skill> { ... }

    /// List all enabled skills.
    pub fn list_enabled(&self) -> Vec<&Skill> { ... }

    /// List all skills (enabled + disabled).
    pub fn list_all(&self) -> Vec<&Skill> { ... }

    /// Add or replace a skill (for runtime creation).
    pub fn insert(&mut self, skill: Skill) { ... }

    /// Remove a skill by name.
    pub fn remove(&mut self, name: &str) -> Option<Skill> { ... }

    /// Generate the `<available_skills>` XML block for system prompt injection.
    pub fn to_prompt_xml(&self) -> String {
        // Compact XML like OpenClaw:
        // <available_skills>
        // <skill name="job-search" description="..." />
        // </available_skills>
    }
}
```

### error.rs

```rust
use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum SkillError {
    #[snafu(display("failed to read skill file: {source}"))]
    Io { source: std::io::Error },
    #[snafu(display("invalid frontmatter: {source}"))]
    Frontmatter { source: serde_yaml::Error },
    #[snafu(display("missing frontmatter in {path}"))]
    MissingFrontmatter { path: String },
    #[snafu(display("invalid trigger regex '{pattern}': {source}"))]
    InvalidTrigger { pattern: String, source: regex::Error },
}
```

### paths additions

```rust
// In crates/paths/src/lib.rs:

/// Returns the path to the user skills directory.
pub fn skills_dir() -> &'static PathBuf {
    static SKILLS_DIR: OnceLock<PathBuf> = OnceLock::new();
    SKILLS_DIR.get_or_init(|| config_dir().join("skills"))
}
```

### Cargo.toml

```toml
[package]
name = "rara-skills"
version.workspace = true
edition.workspace = true

[dependencies]
serde = { workspace = true, features = ["derive"] }
serde_yaml = "0.9"
regex = "1"
snafu = { workspace = true }
tracing = { workspace = true }
```

**Step 1:** Create crate, write `types.rs`, `error.rs`
**Step 2:** Write `loader.rs` with frontmatter parsing + directory scanning
**Step 3:** Write `registry.rs` with trigger matching + prompt XML generation
**Step 4:** Add `skills_dir()` to `rara_paths`
**Step 5:** Write unit tests for frontmatter parsing, trigger matching, precedence
**Step 6:** `cargo check -p rara-skills && cargo test -p rara-skills`
**Step 7:** Commit `feat(skills): add core skill types, loader, and registry (#N)`

---

## Task 2: Integrate skills into ChatService (prompt injection + tool filtering)

**Files:**
- Modify: `crates/agents/src/tool_registry.rs` — add `to_openrouter_tools_filtered()`
- Modify: `crates/domain/chat/src/service.rs` — skill matching + prompt injection + filtered tools
- Modify: `crates/domain/chat/Cargo.toml` — add `rara-skills` dependency
- Modify: `crates/workers/src/worker_state.rs` — initialize `SkillRegistry`, pass to `ChatService`

### ToolRegistry changes

```rust
// In tool_registry.rs, add:

/// Convert only the named tools to OpenRouter format.
/// If `tool_names` is empty, include ALL tools (no filtering).
pub fn to_openrouter_tools_filtered(
    &self,
    tool_names: &[String],
) -> Result<Vec<openrouter_rs::types::Tool>> {
    if tool_names.is_empty() {
        return self.to_openrouter_tools();
    }
    self.tools
        .values()
        .filter(|entry| tool_names.iter().any(|n| n == entry.tool.name()))
        .map(|entry| { /* build Tool */ })
        .collect()
}
```

### ChatService changes

`ChatService` gains a `skill_registry: Arc<RwLock<SkillRegistry>>` field.

In `send_message()`, after composing the system prompt (line ~442):

```rust
// 1. Match skills against user text
let matched_skills = {
    let registry = self.skill_registry.read().unwrap();
    registry.match_triggers(&user_text)
        .into_iter()
        .map(|s| s.clone())
        .collect::<Vec<_>>()
};

// 2. Inject matched skill prompts into system prompt
if !matched_skills.is_empty() {
    for skill in &matched_skills {
        system_prompt.push_str(&format!(
            "\n\n## Active Skill: {}\n\n{}",
            skill.name(), skill.prompt
        ));
    }
}

// 3. Inject available skills listing
{
    let registry = self.skill_registry.read().unwrap();
    let skills_xml = registry.to_prompt_xml();
    if !skills_xml.is_empty() {
        system_prompt.push_str(&format!("\n\n{skills_xml}"));
    }
}

// 4. Collect tool whitelist from matched skills
let tool_whitelist: Vec<String> = matched_skills
    .iter()
    .flat_map(|s| s.tools().iter().cloned())
    .collect();
```

Then when building the runner, pass `tool_whitelist` to filter:

```rust
// Replace `tools.to_openrouter_tools()` with filtered version
let request_tools = if tool_whitelist.is_empty() {
    tools.to_openrouter_tools()?  // no skill matched → all tools
} else {
    tools.to_openrouter_tools_filtered(&tool_whitelist)?
};
```

This requires `AgentRunner` to accept an optional tool filter, OR we build a filtered `ToolRegistry` view.

### worker_state.rs changes

```rust
// After building tool_registry, before building chat_service:
let skill_registry = {
    let user_dir = rara_paths::skills_dir();
    let bundled_dir = std::env::current_dir()
        .unwrap_or_default()
        .join("skills");
    rara_skills::registry::SkillRegistry::load_from_dirs(&[
        bundled_dir.as_path(),
        user_dir.as_path(),
    ])
    .unwrap_or_else(|e| {
        warn!(error = %e, "Failed to load skills, using empty registry");
        rara_skills::registry::SkillRegistry::new()
    })
};
let skill_registry = Arc::new(std::sync::RwLock::new(skill_registry));
```

**Step 1:** Add `to_openrouter_tools_filtered()` to `ToolRegistry`
**Step 2:** Add `skill_registry` field to `ChatService::new()`
**Step 3:** Implement skill matching + prompt injection in `send_message()`
**Step 4:** Wire `SkillRegistry` in `worker_state.rs`
**Step 5:** Write integration test: send message matching a skill trigger, verify tool filtering
**Step 6:** `cargo check -p rara-agents -p rara-domain-chat -p rara-workers`
**Step 7:** Commit `feat(chat): integrate skills into agent loop (#N)`

---

## Task 3: Skill management agent tools

**Files:**
- Create: `crates/workers/src/tools/services/skill_tools.rs`
- Modify: `crates/workers/src/tools/services/mod.rs` — export new tools
- Modify: `crates/workers/src/worker_state.rs` — register skill tools

### Tools to implement

4 tools, all operating on `Arc<RwLock<SkillRegistry>>`:

1. **`list_skills`** — List all skills (name, description, enabled, trigger)
2. **`create_skill`** — Write a new `.md` file to `skills_dir()`, reload registry
   - Params: `{ name, description, tools, trigger, prompt }`
   - Writes formatted SKILL.md to `skills_dir()/{name}.md`
3. **`update_skill`** — Overwrite an existing skill file, reload registry
   - Params: `{ name, description?, tools?, trigger?, prompt?, enabled? }`
4. **`delete_skill`** — Remove a skill file, reload registry
   - Params: `{ name }`

Each tool:
```rust
pub struct CreateSkillTool {
    registry: Arc<RwLock<SkillRegistry>>,
}

#[async_trait]
impl AgentTool for CreateSkillTool {
    fn name(&self) -> &str { "create_skill" }
    fn description(&self) -> &str {
        "Create a new skill. Skills define specialized behaviors with tool whitelists and trigger patterns."
    }
    fn parameters_schema(&self) -> Value { ... }
    async fn execute(&self, params: Value) -> Result<Value> {
        // 1. Parse params
        // 2. Format as SKILL.md with frontmatter
        // 3. Write to skills_dir()/{name}.md
        // 4. Parse the new file
        // 5. Insert into registry
    }
}
```

**Step 1:** Create `skill_tools.rs` with `ListSkillsTool`
**Step 2:** Add `CreateSkillTool`
**Step 3:** Add `UpdateSkillTool` + `DeleteSkillTool`
**Step 4:** Register all 4 tools in `worker_state.rs`
**Step 5:** Unit test: create skill → list → verify → delete → verify gone
**Step 6:** `cargo check -p rara-workers`
**Step 7:** Commit `feat(tools): add skill management agent tools (#N)`

---

## Task 4: HTTP API for skills

**Files:**
- Create: `crates/domain/chat/src/skill_router.rs` (or add to existing `router.rs`)
- Modify: `crates/domain/chat/src/lib.rs` — export skill routes
- Modify: `crates/workers/src/worker_state.rs` — merge skill routes

### Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/skills` | List all skills |
| `GET` | `/api/v1/skills/{name}` | Get skill details |
| `POST` | `/api/v1/skills` | Create a skill |
| `PUT` | `/api/v1/skills/{name}` | Update a skill |
| `DELETE` | `/api/v1/skills/{name}` | Delete a skill |

State: `Arc<RwLock<SkillRegistry>>` as axum state.

```rust
pub fn skill_routes(registry: Arc<RwLock<SkillRegistry>>) -> Router {
    Router::new()
        .route("/api/v1/skills", get(list_skills).post(create_skill))
        .route(
            "/api/v1/skills/{name}",
            get(get_skill).put(update_skill).delete(delete_skill),
        )
        .with_state(registry)
}
```

**Step 1:** Create `skill_router.rs` with list + get handlers
**Step 2:** Add create + update + delete handlers
**Step 3:** Wire routes in `worker_state.rs::routes()`
**Step 4:** `cargo check -p rara-domain-chat -p rara-workers`
**Step 5:** Commit `feat(api): add skills CRUD HTTP endpoints (#N)`

---

## Task 5: Frontend skills page

**Files:**
- Create: `web/src/pages/Skills.tsx`
- Create: `web/src/api/skills.ts` — API client functions
- Modify: `web/src/api/types.ts` — add `Skill` type
- Modify: `web/src/App.tsx` — add route
- Modify: `web/src/components/DashboardLayout.tsx` — add sidebar link

### UI Design

- **List view**: Card grid showing each skill (name, description, enabled badge, tools count)
- **Detail dialog**: View/edit skill markdown with preview
- **Create dialog**: Form with name, description, tools multi-select, trigger pattern, markdown editor
- **Delete**: Confirmation dialog

### API client

```typescript
// web/src/api/skills.ts
export interface Skill {
  name: string;
  description: string;
  tools: string[];
  trigger: string | null;
  enabled: boolean;
  prompt: string;
}

export async function listSkills(): Promise<Skill[]> { ... }
export async function getSkill(name: string): Promise<Skill> { ... }
export async function createSkill(skill: Skill): Promise<Skill> { ... }
export async function updateSkill(name: string, skill: Partial<Skill>): Promise<Skill> { ... }
export async function deleteSkill(name: string): Promise<void> { ... }
```

**Step 1:** Add `Skill` type to `types.ts` and API functions to `skills.ts`
**Step 2:** Create `Skills.tsx` with list view + create/edit dialogs
**Step 3:** Add route in `App.tsx` and sidebar link in `DashboardLayout.tsx`
**Step 4:** `cd web && npm run build`
**Step 5:** Commit `feat(web): add skills management page (#N)`

---

## Task 6: Bundled default skills

**Files:**
- Create: `skills/job-search.md`
- Create: `skills/resume-review.md`
- Create: `skills/interview-prep.md`
- Create: `skills/coding-agent.md`

Provide 3-4 bundled skills that showcase the system:

1. **job-search** — Tools: `job_pipeline`, `memory_search`, `http_fetch`. Trigger: `找工作|job search|搜索职位`
2. **resume-review** — Tools: `list_resumes`, `get_resume_content`, `analyze_resume`, `memory_search`. Trigger: `简历|resume|CV`
3. **interview-prep** — Tools: `memory_search`, `memory_write`, `http_fetch`. Trigger: `面试|interview|prep`
4. **coding-agent** — Tools: `codex_run`, `codex_status`, `codex_list`, `bash`, `read_file`, `write_file`. Trigger: `写代码|code|implement|开发`

**Step 1:** Write each skill file with appropriate frontmatter + prompt
**Step 2:** Verify they parse correctly with `rara-skills` loader
**Step 3:** Commit `feat(skills): add bundled default skills (#N)`

---

## Task dependency graph

```
Task 1 (core crate)
  ├── Task 2 (chat integration) ← depends on Task 1
  │     └── Task 6 (bundled skills) ← depends on Task 2
  ├── Task 3 (agent tools) ← depends on Task 1
  ├── Task 4 (HTTP API) ← depends on Task 1
  └── Task 5 (frontend) ← depends on Task 4
```

Tasks 2, 3, 4 can run in parallel after Task 1.
Task 5 depends on Task 4 (needs API).
Task 6 depends on Task 2 (needs integration to verify).

---

## Issue mapping

| Issue | Title | Labels | Dependencies |
|-------|-------|--------|-------------|
| #A | `feat(skills): core skill types, loader, and registry crate` | `created-by:claude`, `enhancement`, `backend` | none |
| #B | `feat(chat): integrate skills into agent loop (prompt + tool filtering)` | `created-by:claude`, `enhancement`, `backend` | #A |
| #C | `feat(tools): add skill management agent tools` | `created-by:claude`, `enhancement`, `backend` | #A |
| #D | `feat(api): add skills CRUD HTTP endpoints` | `created-by:claude`, `enhancement`, `backend` | #A |
| #E | `feat(web): add skills management page` | `created-by:claude`, `enhancement`, `ui` | #D |
| #F | `feat(skills): add bundled default skills` | `created-by:claude`, `enhancement` | #B |
