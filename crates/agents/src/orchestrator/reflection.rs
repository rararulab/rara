use std::sync::Arc;

use rara_memory::MemoryManager;

use agent_core::{
    model::LlmProviderLoaderRef,
    runner::{AgentRunner, UserContent},
    tool_registry::ToolRegistry,
};

use super::error::OrchestratorError;

/// Run a lightweight memory reflection after a conversation turn.
pub async fn memory_reflection(
    _mm: &Arc<MemoryManager>,
    llm: &LlmProviderLoaderRef,
    tools: &Arc<ToolRegistry>,
    model: &str,
    user_text: &str,
    assistant_text: &str,
) -> Result<(), OrchestratorError> {
    let mut reflection_tools = ToolRegistry::default();
    if let Some(tool) = tools.get("memory_update_profile") {
        reflection_tools.register_service(Arc::clone(tool));
    }
    if let Some(tool) = tools.get("memory_write") {
        reflection_tools.register_service(Arc::clone(tool));
    }
    if reflection_tools.is_empty() {
        return Ok(());
    }

    let reflection_prompt = format!(
        "You are a memory maintenance agent. Based on the following exchange, extract any new \
         facts about the user (name, role, location, preferences, goals, important context). If \
         you learned something new, use memory_update_profile to update the relevant section \
         (\"Basic Info\", \"Preferences\", \"Current Goals\", or \"Key Context\"). If nothing new \
         was learned, do nothing — do NOT call any tools.\n\nKeep updates concise (3-5 bullet \
         points per section max). Only add genuinely useful information.\n\n## User \
         Message\n{user_text}\n\n## Assistant Response\n{assistant_text}"
    );

    let runner = AgentRunner::builder()
        .llm_provider(llm.clone())
        .model_name(model.to_owned())
        .system_prompt(
            "You are a silent memory maintenance agent. Your only job is to update the user \
             profile if new facts were learned. Never produce conversational output."
                .to_owned(),
        )
        .user_content(UserContent::Text(reflection_prompt))
        .max_iterations(1_usize)
        .build();

    let _result = runner
        .run(&reflection_tools, None)
        .await
        .map_err(|e| OrchestratorError::AgentError {
            message: format!("memory reflection agent: {e}"),
        })?;

    tracing::debug!("memory reflection complete");
    Ok(())
}
