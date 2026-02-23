# Sub-Agent Chain Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `subagent` tool that lets the main agent orchestrate sub-agents in single, chain, and parallel modes — following pi-mono's lightweight approach.

**Architecture:** Sub-agents are just new `AgentRunner` instances created in-process. Agent definitions are markdown files with YAML frontmatter (like skills). A `SubagentTool` implements `AgentTool` and handles single/chain/parallel dispatch. No DAG framework, no process isolation — the LLM decides when and how to use the subagent tool.

**Tech Stack:** `rara-agents` (AgentRunner, ToolRegistry), `tool-core` (AgentTool trait), `serde_yaml` (frontmatter parsing), `tokio` (concurrent execution)

---

### Task 1: Agent Definition Types + Parser

**Files:**
- Create: `crates/agents/src/subagent/mod.rs`
- Create: `crates/agents/src/subagent/definition.rs`
- Modify: `crates/agents/src/lib.rs` (add `pub mod subagent;`)

**Step 1: Write the failing test for AgentDefinition parsing**

In `crates/agents/src/subagent/definition.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_definition() {
        let content = r#"---
name: scout
description: "Fast codebase recon"
model: "deepseek/deepseek-chat"
tools:
  - read_file
  - grep
  - find_files
max_iterations: 15
---

You are a scout. Quickly investigate and return structured findings.
"#;
        let def = AgentDefinition::parse(content).unwrap();
        assert_eq!(def.name, "scout");
        assert_eq!(def.description, "Fast codebase recon");
        assert_eq!(def.model.as_deref(), Some("deepseek/deepseek-chat"));
        assert_eq!(def.tools, vec!["read_file", "grep", "find_files"]);
        assert_eq!(def.max_iterations, Some(15));
        assert!(def.system_prompt.contains("You are a scout"));
    }

    #[test]
    fn parse_minimal_definition() {
        let content = "---\nname: worker\ndescription: General worker\n---\nDo the work.\n";
        let def = AgentDefinition::parse(content).unwrap();
        assert_eq!(def.name, "worker");
        assert!(def.model.is_none());
        assert!(def.tools.is_empty());
        assert!(def.max_iterations.is_none());
    }

    #[test]
    fn parse_missing_frontmatter_fails() {
        let content = "# No frontmatter\nJust markdown.";
        assert!(AgentDefinition::parse(content).is_err());
    }

    #[test]
    fn registry_load_and_get() {
        let content = "---\nname: scout\ndescription: Recon\n---\nPrompt.\n";
        let mut registry = AgentDefinitionRegistry::new();
        registry.register(AgentDefinition::parse(content).unwrap());
        assert!(registry.get("scout").is_some());
        assert!(registry.get("nonexistent").is_none());
        assert_eq!(registry.list().len(), 1);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-agents subagent::definition::tests`
Expected: FAIL — module doesn't exist yet

**Step 3: Implement AgentDefinition and AgentDefinitionRegistry**

`crates/agents/src/subagent/mod.rs`:
```rust
pub mod definition;
mod executor;
mod tool;

pub use definition::{AgentDefinition, AgentDefinitionRegistry};
pub use tool::SubagentTool;
```

`crates/agents/src/subagent/definition.rs`:
```rust
use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::err::prelude::*;

/// YAML frontmatter from an agent definition markdown file.
#[derive(Debug, Clone, Deserialize)]
struct AgentFrontmatter {
    name:           String,
    #[serde(default)]
    description:    String,
    #[serde(default)]
    model:          Option<String>,
    #[serde(default)]
    tools:          Vec<String>,
    #[serde(default)]
    max_iterations: Option<usize>,
}

/// A parsed agent definition: frontmatter metadata + system prompt body.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub name:           String,
    pub description:    String,
    pub model:          Option<String>,
    pub tools:          Vec<String>,
    pub max_iterations: Option<usize>,
    pub system_prompt:  String,
}

impl AgentDefinition {
    /// Parse a markdown string with YAML frontmatter into an AgentDefinition.
    pub fn parse(content: &str) -> Result<Self> {
        let (frontmatter, body) = split_frontmatter(content)?;
        let meta: AgentFrontmatter =
            serde_yaml::from_str(&frontmatter).map_err(|e| Error::Other {
                message: format!("invalid agent definition frontmatter: {e}").into(),
            })?;
        Ok(Self {
            name:           meta.name,
            description:    meta.description,
            model:          meta.model,
            tools:          meta.tools,
            max_iterations: meta.max_iterations,
            system_prompt:  body,
        })
    }
}

/// Registry holding named agent definitions.
#[derive(Debug, Clone, Default)]
pub struct AgentDefinitionRegistry {
    defs: HashMap<String, AgentDefinition>,
}

impl AgentDefinitionRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, def: AgentDefinition) {
        self.defs.insert(def.name.clone(), def);
    }

    pub fn get(&self, name: &str) -> Option<&AgentDefinition> {
        self.defs.get(name)
    }

    pub fn list(&self) -> Vec<&AgentDefinition> {
        self.defs.values().collect()
    }

    /// Load all `.md` files from a directory as agent definitions.
    pub fn load_dir(dir: &Path) -> Result<Self> {
        let mut registry = Self::new();
        if !dir.is_dir() {
            return Ok(registry);
        }
        let entries = std::fs::read_dir(dir).map_err(|e| Error::IO {
            source: e,
            location: snafu::Location::new(file!(), line!(), 0),
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md") {
                let content = std::fs::read_to_string(&path).map_err(|e| Error::IO {
                    source: e,
                    location: snafu::Location::new(file!(), line!(), 0),
                })?;
                match AgentDefinition::parse(&content) {
                    Ok(def) => { registry.register(def); }
                    Err(err) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %err,
                            "skipping invalid agent definition"
                        );
                    }
                }
            }
        }
        Ok(registry)
    }
}

/// Split markdown content at `---` delimiters into (frontmatter, body).
fn split_frontmatter(content: &str) -> Result<(String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err(Error::Other {
            message: "agent definition missing frontmatter (must start with ---)".into(),
        });
    }
    let after_open = &trimmed[3..];
    let close_pos = after_open.find("\n---").ok_or_else(|| Error::Other {
        message: "agent definition missing closing --- delimiter".into(),
    })?;
    let frontmatter = after_open[..close_pos].trim().to_string();
    let body = after_open[close_pos + 4..].trim().to_string();
    Ok((frontmatter, body))
}
```

Add to `crates/agents/src/lib.rs`:
```rust
pub mod subagent;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p rara-agents subagent::definition::tests`
Expected: all 4 tests PASS

**Step 5: Commit**

```bash
git add crates/agents/src/subagent/ crates/agents/src/lib.rs
git commit -m "feat(agents): add AgentDefinition types and parser (#N)"
```

---

### Task 2: Executor Logic (single/chain/parallel)

**Files:**
- Create: `crates/agents/src/subagent/executor.rs`

**Step 1: Write the failing test for executor output extraction**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_output_from_response() {
        let text = "Here is my analysis of the codebase.";
        assert_eq!(extract_final_text(text), text);
    }

    #[test]
    fn chain_replaces_previous_placeholder() {
        let task = "Create a plan based on: {previous}";
        let previous = "Found 3 auth files.";
        let result = task.replace("{previous}", previous);
        assert_eq!(result, "Create a plan based on: Found 3 auth files.");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-agents subagent::executor::tests`
Expected: FAIL

**Step 3: Implement the executor**

`crates/agents/src/subagent/executor.rs`:
```rust
use std::sync::Arc;

use base::shared_string::SharedString;
use tracing::{info, warn};

use crate::{
    err::prelude::*,
    model::LlmProviderLoaderRef,
    runner::{AgentRunResponse, AgentRunner, UserContent, MAX_ITERATIONS},
    tool_registry::ToolRegistry,
};
use super::definition::{AgentDefinition, AgentDefinitionRegistry};

/// Result from running a single sub-agent.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubagentResult {
    pub agent_name: String,
    pub output:     String,
    pub iterations: usize,
    pub tool_calls: usize,
    pub success:    bool,
    pub error:      Option<String>,
}

/// Execute a single sub-agent.
pub async fn run_single(
    def: &AgentDefinition,
    task: &str,
    llm_provider: &LlmProviderLoaderRef,
    parent_tools: &ToolRegistry,
    default_model: &str,
) -> SubagentResult {
    let model = def.model.as_deref().unwrap_or(default_model);
    let max_iter = def.max_iterations.unwrap_or(15);

    // Build filtered tool registry: only tools listed in the agent definition.
    // Always exclude "subagent" to prevent recursive calls.
    let tools = if def.tools.is_empty() {
        let all_names: Vec<String> = parent_tools
            .tool_names()
            .into_iter()
            .filter(|n| n != "subagent")
            .collect();
        parent_tools.filtered(&all_names)
    } else {
        let names: Vec<String> = def.tools.iter()
            .filter(|n| n.as_str() != "subagent")
            .cloned()
            .collect();
        parent_tools.filtered(&names)
    };

    info!(
        agent = %def.name,
        model,
        max_iterations = max_iter,
        tool_count = tools.len(),
        "running sub-agent"
    );

    let runner = AgentRunner::builder()
        .llm_provider(Arc::clone(llm_provider))
        .model_name(model)
        .system_prompt(&def.system_prompt)
        .user_content(UserContent::Text(task.to_owned()))
        .max_iterations(max_iter)
        .build();

    match runner.run(&tools, None).await {
        Ok(response) => {
            let text = response
                .provider_response
                .choices
                .first()
                .and_then(|c| c.message.content.as_deref())
                .unwrap_or("")
                .to_owned();
            SubagentResult {
                agent_name: def.name.clone(),
                output:     text,
                iterations: response.iterations,
                tool_calls: response.tool_calls_made,
                success:    true,
                error:      None,
            }
        }
        Err(err) => {
            warn!(agent = %def.name, error = %err, "sub-agent failed");
            SubagentResult {
                agent_name: def.name.clone(),
                output:     String::new(),
                iterations: 0,
                tool_calls: 0,
                success:    false,
                error:      Some(err.to_string()),
            }
        }
    }
}

/// Execute a chain of sub-agents sequentially.
/// Each step's task can reference `{previous}` to get the prior step's output.
pub async fn run_chain(
    steps: &[(String, String)],  // (agent_name, task)
    registry: &AgentDefinitionRegistry,
    llm_provider: &LlmProviderLoaderRef,
    parent_tools: &ToolRegistry,
    default_model: &str,
) -> Vec<SubagentResult> {
    let mut results = Vec::with_capacity(steps.len());
    let mut previous_output = String::new();

    for (i, (agent_name, task_template)) in steps.iter().enumerate() {
        let Some(def) = registry.get(agent_name) else {
            results.push(SubagentResult {
                agent_name: agent_name.clone(),
                output:     String::new(),
                iterations: 0,
                tool_calls: 0,
                success:    false,
                error:      Some(format!("agent definition not found: {agent_name}")),
            });
            break;
        };

        let task = task_template.replace("{previous}", &previous_output);

        info!(
            step = i + 1,
            total = steps.len(),
            agent = %agent_name,
            "running chain step"
        );

        let result = run_single(def, &task, llm_provider, parent_tools, default_model).await;

        if !result.success {
            results.push(result);
            break; // Chain stops on first error
        }

        previous_output = result.output.clone();
        results.push(result);
    }

    results
}

/// Execute multiple sub-agents in parallel with a concurrency limit.
pub async fn run_parallel(
    tasks: &[(String, String)],  // (agent_name, task)
    registry: &AgentDefinitionRegistry,
    llm_provider: &LlmProviderLoaderRef,
    parent_tools: &ToolRegistry,
    default_model: &str,
    max_concurrency: usize,
) -> Vec<SubagentResult> {
    use tokio::sync::Semaphore;

    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    let mut handles = Vec::with_capacity(tasks.len());

    for (agent_name, task) in tasks {
        let sem = Arc::clone(&semaphore);
        let llm = Arc::clone(llm_provider);
        let def = registry.get(agent_name).cloned();
        let task = task.clone();
        let agent_name = agent_name.clone();
        let default_model = default_model.to_owned();

        // We need to clone the filtered tools for each parallel task.
        let tools = if let Some(ref d) = def {
            if d.tools.is_empty() {
                let all_names: Vec<String> = parent_tools
                    .tool_names()
                    .into_iter()
                    .filter(|n| n != "subagent")
                    .collect();
                parent_tools.filtered(&all_names)
            } else {
                let names: Vec<String> = d.tools.iter()
                    .filter(|n| n.as_str() != "subagent")
                    .cloned()
                    .collect();
                parent_tools.filtered(&names)
            }
        } else {
            ToolRegistry::default()
        };

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            match def {
                Some(d) => {
                    run_single(&d, &task, &llm, &tools, &default_model).await
                }
                None => SubagentResult {
                    agent_name,
                    output:     String::new(),
                    iterations: 0,
                    tool_calls: 0,
                    success:    false,
                    error:      Some("agent definition not found".to_string()),
                },
            }
        }));
    }

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(err) => results.push(SubagentResult {
                agent_name: "unknown".to_string(),
                output:     String::new(),
                iterations: 0,
                tool_calls: 0,
                success:    false,
                error:      Some(format!("task panicked: {err}")),
            }),
        }
    }
    results
}

/// Extract the final assistant text from an LLM response (helper).
pub fn extract_final_text(text: &str) -> &str {
    text.trim()
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p rara-agents subagent::executor::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/agents/src/subagent/executor.rs
git commit -m "feat(agents): add subagent executor for single/chain/parallel (#N)"
```

---

### Task 3: SubagentTool Implementation

**Files:**
- Create: `crates/agents/src/subagent/tool.rs`

**Step 1: Write the failing test for parameter parsing**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_params() {
        let json = serde_json::json!({
            "agent": "scout",
            "task": "Find auth code"
        });
        let params: SubagentParams = serde_json::from_value(json).unwrap();
        assert!(matches!(params, SubagentParams::Single { .. }));
    }

    #[test]
    fn parse_chain_params() {
        let json = serde_json::json!({
            "chain": [
                { "agent": "scout", "task": "Find code" },
                { "agent": "planner", "task": "Plan based on {previous}" }
            ]
        });
        let params: SubagentParams = serde_json::from_value(json).unwrap();
        assert!(matches!(params, SubagentParams::Chain { .. }));
    }

    #[test]
    fn parse_parallel_params() {
        let json = serde_json::json!({
            "parallel": [
                { "agent": "scout", "task": "Find models" },
                { "agent": "scout", "task": "Find providers" }
            ],
            "max_concurrency": 2
        });
        let params: SubagentParams = serde_json::from_value(json).unwrap();
        assert!(matches!(params, SubagentParams::Parallel { .. }));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-agents subagent::tool::tests`
Expected: FAIL

**Step 3: Implement SubagentTool**

`crates/agents/src/subagent/tool.rs`:
```rust
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    model::LlmProviderLoaderRef,
    tool_registry::ToolRegistry,
};
use super::{
    definition::AgentDefinitionRegistry,
    executor,
};

/// Maximum number of parallel sub-agent tasks.
const MAX_PARALLEL_TASKS: usize = 8;
/// Default concurrency for parallel mode.
const DEFAULT_CONCURRENCY: usize = 4;

/// A single sub-agent task step: agent name + task description.
#[derive(Debug, Clone, Deserialize)]
pub struct SubagentStep {
    pub agent: String,
    pub task:  String,
}

/// Parameters for the subagent tool. Supports three modes:
/// - Single: run one sub-agent
/// - Chain: run sub-agents sequentially, passing output via `{previous}`
/// - Parallel: run sub-agents concurrently
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SubagentParams {
    Chain {
        chain: Vec<SubagentStep>,
    },
    Parallel {
        parallel:        Vec<SubagentStep>,
        #[serde(default)]
        max_concurrency: Option<usize>,
    },
    Single {
        agent: String,
        task:  String,
    },
}

/// Tool that spawns sub-agents with isolated contexts.
pub struct SubagentTool {
    llm_provider:  LlmProviderLoaderRef,
    definitions:   Arc<AgentDefinitionRegistry>,
    parent_tools:  Arc<ToolRegistry>,
    default_model: String,
}

impl SubagentTool {
    pub fn new(
        llm_provider:  LlmProviderLoaderRef,
        definitions:   Arc<AgentDefinitionRegistry>,
        parent_tools:  Arc<ToolRegistry>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            llm_provider,
            definitions,
            parent_tools,
            default_model: default_model.into(),
        }
    }
}

#[async_trait]
impl tool_core::AgentTool for SubagentTool {
    fn name(&self) -> &str { "subagent" }

    fn description(&self) -> &str {
        "Run sub-agents to handle complex tasks. Supports three modes:\n\
         1. Single: {\"agent\": \"<name>\", \"task\": \"<description>\"}\n\
         2. Chain: {\"chain\": [{\"agent\": \"<name>\", \"task\": \"...\"}]} — sequential, use {previous} to reference prior output\n\
         3. Parallel: {\"parallel\": [{\"agent\": \"<name>\", \"task\": \"...\"}]} — concurrent execution"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        // Build available agents list for the description
        let agents: Vec<String> = self.definitions.list().iter()
            .map(|d| format!("{}: {}", d.name, d.description))
            .collect();
        let agents_desc = if agents.is_empty() {
            "No agents defined".to_string()
        } else {
            agents.join(", ")
        };

        serde_json::json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": format!("Agent name for single mode. Available: {agents_desc}")
                },
                "task": {
                    "type": "string",
                    "description": "Task description for the agent"
                },
                "chain": {
                    "type": "array",
                    "description": "Sequential chain of agent tasks. Use {previous} in task to reference prior output.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent": { "type": "string" },
                            "task": { "type": "string" }
                        },
                        "required": ["agent", "task"]
                    }
                },
                "parallel": {
                    "type": "array",
                    "description": "Parallel agent tasks. Results are aggregated.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent": { "type": "string" },
                            "task": { "type": "string" }
                        },
                        "required": ["agent", "task"]
                    }
                },
                "max_concurrency": {
                    "type": "integer",
                    "description": "Max concurrent tasks for parallel mode (default: 4, max: 8)"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let params: SubagentParams = serde_json::from_value(params)?;

        match params {
            SubagentParams::Single { agent, task } => {
                let def = self.definitions.get(&agent)
                    .ok_or_else(|| anyhow::anyhow!(
                        "agent '{}' not found. Available: {}",
                        agent,
                        self.definitions.list().iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ")
                    ))?;
                let result = executor::run_single(
                    def, &task, &self.llm_provider, &self.parent_tools, &self.default_model,
                ).await;
                Ok(serde_json::to_value(&result)?)
            }

            SubagentParams::Chain { chain } => {
                if chain.is_empty() {
                    anyhow::bail!("chain must have at least one step");
                }
                let steps: Vec<(String, String)> = chain.into_iter()
                    .map(|s| (s.agent, s.task))
                    .collect();
                let results = executor::run_chain(
                    &steps, &self.definitions, &self.llm_provider,
                    &self.parent_tools, &self.default_model,
                ).await;
                Ok(serde_json::to_value(&results)?)
            }

            SubagentParams::Parallel { parallel, max_concurrency } => {
                if parallel.is_empty() {
                    anyhow::bail!("parallel must have at least one task");
                }
                if parallel.len() > MAX_PARALLEL_TASKS {
                    anyhow::bail!("too many parallel tasks (max {})", MAX_PARALLEL_TASKS);
                }
                let concurrency = max_concurrency
                    .unwrap_or(DEFAULT_CONCURRENCY)
                    .min(DEFAULT_CONCURRENCY);
                let tasks: Vec<(String, String)> = parallel.into_iter()
                    .map(|s| (s.agent, s.task))
                    .collect();
                let results = executor::run_parallel(
                    &tasks, &self.definitions, &self.llm_provider,
                    &self.parent_tools, &self.default_model, concurrency,
                ).await;
                Ok(serde_json::to_value(&results)?)
            }
        }
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p rara-agents subagent::tool::tests`
Expected: PASS

**Step 5: Run cargo check**

Run: `cargo check -p rara-agents`
Expected: compiles clean

**Step 6: Commit**

```bash
git add crates/agents/src/subagent/tool.rs
git commit -m "feat(agents): add SubagentTool with single/chain/parallel modes (#N)"
```

---

### Task 4: Agent Definition Files

**Files:**
- Create: `agents/scout.md`
- Create: `agents/planner.md`
- Create: `agents/worker.md`

**Step 1: Create scout agent**

`agents/scout.md`:
```markdown
---
name: scout
description: "快速代码侦察，返回结构化分析结果"
model: "deepseek/deepseek-chat"
tools:
  - read_file
  - grep
  - find_files
  - list_directory
  - http_fetch
max_iterations: 15
---

You are a scout agent. Your job is to quickly investigate a codebase or topic and return compressed, structured findings.

## Output Format

### Files Found
- `path/to/file.ext` (lines N-M) — Brief description

### Key Code
Relevant code snippets with context.

### Architecture
Brief explanation of how things connect.

### Summary
2-3 sentence summary of findings.
```

**Step 2: Create planner agent**

`agents/planner.md`:
```markdown
---
name: planner
description: "根据调查结果制定实施方案"
tools:
  - read_file
  - grep
  - find_files
max_iterations: 10
---

You are a planner agent. Given investigation results from a scout, create a clear implementation plan.

## Output Format

### Goal
One sentence describing the objective.

### Steps
1. **Step title** — What to do, which files to touch.
2. ...

### Risks
Any concerns or edge cases to watch for.
```

**Step 3: Create worker agent**

`agents/worker.md`:
```markdown
---
name: worker
description: "按照计划执行具体实现任务"
tools:
  - read_file
  - write_file
  - edit_file
  - bash
  - grep
  - find_files
max_iterations: 20
---

You are a worker agent. Given an implementation plan, execute it step by step.

- Make minimal, focused changes.
- Test your work after each step.
- Report what you changed and the result.
```

**Step 4: Commit**

```bash
git add agents/
git commit -m "feat(agents): add scout/planner/worker agent definitions (#N)"
```

---

### Task 5: Integration in Composition Root

**Files:**
- Modify: `crates/workers/src/worker_state.rs` — register SubagentTool

**Step 1: Load agent definitions and register SubagentTool**

In `crates/workers/src/worker_state.rs`, after the existing tool registration block (around line 200+), add:

```rust
// Load sub-agent definitions
let agent_defs = {
    let agents_dir = rara_paths::data_dir().join("agents");
    let bundled_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../agents");
    let mut registry = rara_agents::subagent::AgentDefinitionRegistry::new();
    // Load bundled agent definitions first
    if let Ok(bundled) = rara_agents::subagent::AgentDefinitionRegistry::load_dir(&bundled_dir) {
        for def in bundled.list() {
            registry.register(def.clone());
        }
    }
    // User-defined agents override bundled ones
    if let Ok(user) = rara_agents::subagent::AgentDefinitionRegistry::load_dir(&agents_dir) {
        for def in user.list() {
            registry.register(def.clone());
        }
    }
    Arc::new(registry)
};

// Get default model for sub-agents
let default_subagent_model = {
    let settings = settings_svc.current();
    settings.ai.model_for(rara_domain_shared::settings::ModelScenario::Chat).to_owned()
};

// Register subagent tool
tool_registry.register_service(Arc::new(
    rara_agents::subagent::SubagentTool::new(
        llm_provider.clone(),
        agent_defs,
        Arc::new(tool_registry.filtered(&[])),  // snapshot of current tools
        default_subagent_model,
    ),
));
```

> **Note:** The exact location and variable names may differ — adapt to match the current code at the insertion point. The key requirement is:
> 1. SubagentTool must be registered AFTER all other tools (so it can snapshot them)
> 2. The parent_tools snapshot must NOT include SubagentTool itself (it's registered after the snapshot)

**Step 2: Verify compilation**

Run: `cargo check -p rara-workers`
Expected: compiles clean

**Step 3: Commit**

```bash
git add crates/workers/src/worker_state.rs
git commit -m "feat(workers): register SubagentTool in composition root (#N)"
```

---

### Task 6: Cargo.toml Dependencies

**Files:**
- Modify: `crates/agents/Cargo.toml` — ensure `serde_yaml` is available

**Step 1: Check and add serde_yaml if needed**

Verify `serde_yaml` is in workspace dependencies. If not, add to `crates/agents/Cargo.toml`:

```toml
serde_yaml.workspace = true
```

Or if not in workspace, add directly:
```toml
serde_yaml = "0.9"
```

**Step 2: Verify full build**

Run: `cargo check -p rara-agents && cargo check -p rara-workers`
Expected: compiles clean

**Step 3: Commit if changes were needed**

```bash
git add crates/agents/Cargo.toml Cargo.toml
git commit -m "chore(agents): add serde_yaml dependency (#N)"
```

---

### Task 7: End-to-End Verification

**Step 1: Run all agent tests**

Run: `cargo test -p rara-agents`
Expected: all tests PASS (definition parsing + tool param parsing)

**Step 2: Run full workspace check**

Run: `cargo check --workspace`
Expected: compiles clean

**Step 3: Final commit with all remaining changes**

```bash
git add -A
git commit -m "feat(agents): subagent chain — single/chain/parallel orchestration (#N)

Adds a SubagentTool that lets the main agent orchestrate sub-agents:
- Single mode: run one sub-agent with isolated context
- Chain mode: sequential execution with {previous} output passing
- Parallel mode: concurrent execution with semaphore-based concurrency limit

Agent definitions are markdown files with YAML frontmatter (like skills).
Three built-in agents: scout, planner, worker.

Closes #N"
```

---

## Design Decisions

1. **In-process, not process isolation** — Sub-agents run as new `AgentRunner` instances in the same process. Simpler, faster, no IPC overhead. Recursive calls prevented by filtering out "subagent" from sub-agent tool registries.

2. **Agent definitions are markdown** — Reuses the same frontmatter pattern as skills. Easy to create, edit, and version control.

3. **Orchestration by LLM** — The main agent decides when/how to use the subagent tool. No rigid DAG — the three primitives (single/chain/parallel) are sufficient for the LLM to compose complex workflows.

4. **Chain stops on error** — Like pi-mono: if any step in a chain fails, the entire chain stops immediately and returns partial results.

5. **Concurrency limit** — Parallel mode defaults to 4 concurrent tasks, max 8. Prevents overloading the LLM provider.
