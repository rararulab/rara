use std::sync::Arc;

use tracing::{info, warn};

use crate::{
    model::LlmProviderLoaderRef,
    runner::{AgentRunner, UserContent},
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

/// Build a filtered ToolRegistry for a sub-agent, always excluding "subagent".
fn build_subagent_tools(def: &AgentDefinition, parent_tools: &ToolRegistry) -> ToolRegistry {
    if def.tools.is_empty() {
        let all_names: Vec<String> = parent_tools
            .tool_names()
            .into_iter()
            .filter(|n| n != "subagent")
            .collect();
        parent_tools.filtered(&all_names)
    } else {
        let names: Vec<String> = def
            .tools
            .iter()
            .filter(|n| n.as_str() != "subagent")
            .cloned()
            .collect();
        parent_tools.filtered(&names)
    }
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
    let tools = build_subagent_tools(def, parent_tools);

    info!(
        agent = %def.name,
        model,
        max_iterations = max_iter,
        tool_count = tools.len(),
        "running sub-agent"
    );

    let runner = AgentRunner::builder()
        .llm_provider(Arc::clone(llm_provider))
        .model_name(model.to_owned())
        .system_prompt(def.system_prompt.clone())
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
    steps: &[(String, String)], // (agent_name, task)
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

        info!(step = i + 1, total = steps.len(), agent = %agent_name, "running chain step");

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
    tasks: &[(String, String)], // (agent_name, task)
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
        let tools = match def {
            Some(ref d) => build_subagent_tools(d, parent_tools),
            None => ToolRegistry::default(),
        };

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            match def {
                Some(d) => run_single(&d, &task, &llm, &tools, &default_model).await,
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
