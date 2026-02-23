use rara_domain_shared::settings::model::Settings;

/// Resolve the soul prompt from runtime settings or on-disk file.
pub fn resolve_soul_prompt(settings: &Settings) -> Option<String> {
    if settings
        .agent
        .soul
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        return settings.agent.soul.clone();
    }
    let markdown_soul = rara_paths::load_agent_soul_prompt();
    if markdown_soul.trim().is_empty() {
        return None;
    }
    Some(markdown_soul)
}

/// Compose a chat system prompt, optionally prepending a soul document.
pub fn compose_system_prompt(base_prompt: &str, soul_prompt: Option<&str>) -> String {
    if let Some(soul) = soul_prompt.filter(|s| !s.trim().is_empty()) {
        if base_prompt.contains(soul.trim()) {
            return base_prompt.to_owned();
        }
        return format!("{soul}\n\n# Chat Instructions\n{base_prompt}");
    }
    base_prompt.to_owned()
}

/// Compose a worker operational policy, optionally prepending a soul document.
pub fn compose_policy(base_policy: &str, soul_prompt: Option<&str>) -> String {
    if let Some(soul) = soul_prompt.filter(|s| !s.trim().is_empty()) {
        return format!("{soul}\n\n# Operational Policy\n{base_policy}");
    }
    base_policy.to_owned()
}

/// Load the agent behavioral policy from on-disk file or built-in default.
pub async fn load_agent_policy(settings: &Settings) -> String {
    const DEFAULT_AGENT_POLICY: &str =
        include_str!("../../../../prompts/workers/agent_policy.md");

    let soul_prompt = resolve_soul_prompt(settings);
    let policy_path = rara_paths::agent_policy_file();
    if let Ok(content) = tokio::fs::read_to_string(policy_path).await {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return compose_policy(trimmed, soul_prompt.as_deref());
        }
    }
    let prompt_content =
        rara_paths::load_prompt_markdown("workers/agent_policy.md", DEFAULT_AGENT_POLICY);
    if !prompt_content.trim().is_empty() {
        return compose_policy(&prompt_content, soul_prompt.as_deref());
    }
    compose_policy(DEFAULT_AGENT_POLICY, soul_prompt.as_deref())
}
