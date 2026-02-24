use crate::PromptRepo;

/// Compose a base prompt with an optional soul/personality prefix.
///
/// If `soul` is provided and non-empty, it is prepended to `base`
/// with `section_title` as a heading separator. If `base` already
/// contains the soul text, the base is returned as-is to avoid duplication.
#[must_use]
pub fn compose_with_soul(base: &str, soul: Option<&str>, section_title: &str) -> String {
    if let Some(soul) = soul.filter(|s| !s.trim().is_empty()) {
        if base.contains(soul.trim()) {
            return base.to_owned();
        }
        return format!("{soul}\n\n# {section_title}\n{base}");
    }
    base.to_owned()
}

/// Resolve the effective soul prompt.
///
/// Priority: `settings_soul` override > repo file content (`"agent/soul.md"`).
pub async fn resolve_soul(
    repo: &dyn PromptRepo,
    settings_soul: Option<&str>,
) -> Option<String> {
    if let Some(soul) = settings_soul.filter(|s| !s.trim().is_empty()) {
        return Some(soul.to_owned());
    }
    repo.get("agent/soul.md")
        .await
        .map(|e| e.content)
        .filter(|c| !c.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PromptEntry, PromptError};

    // ---------------------------------------------------------------
    // compose_with_soul tests
    // ---------------------------------------------------------------

    #[test]
    fn compose_without_soul_returns_base() {
        let base = "You are a helpful assistant.";
        let result = compose_with_soul(base, None, "Instructions");
        assert_eq!(result, base);
    }

    #[test]
    fn compose_with_empty_soul_returns_base() {
        let base = "You are a helpful assistant.";
        let result = compose_with_soul(base, Some("   "), "Instructions");
        assert_eq!(result, base);
    }

    #[test]
    fn compose_with_soul_prepends() {
        let base = "Analyze the job description.";
        let soul = "You are Rara, a job-hunting AI.";
        let result = compose_with_soul(base, Some(soul), "Task Instructions");
        assert_eq!(
            result,
            "You are Rara, a job-hunting AI.\n\n# Task Instructions\nAnalyze the job description."
        );
    }

    #[test]
    fn compose_deduplicates_when_base_contains_soul() {
        let soul = "You are Rara.";
        let base = "You are Rara.\n\nDo the thing.";
        let result = compose_with_soul(base, Some(soul), "Instructions");
        assert_eq!(result, base);
    }

    // ---------------------------------------------------------------
    // resolve_soul tests
    // ---------------------------------------------------------------

    /// A simple in-memory `PromptRepo` for testing.
    struct MockRepo {
        soul: Option<PromptEntry>,
    }

    #[async_trait::async_trait]
    impl PromptRepo for MockRepo {
        async fn get(&self, name: &str) -> Option<PromptEntry> {
            if name == "agent/soul.md" {
                return self.soul.clone();
            }
            None
        }

        async fn list(&self) -> Vec<PromptEntry> {
            self.soul.iter().cloned().collect()
        }

        async fn update(&self, _name: &str, _content: &str) -> Result<PromptEntry, PromptError> {
            unimplemented!()
        }

        async fn reset(&self, _name: &str) -> Result<PromptEntry, PromptError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn resolve_soul_prefers_settings() {
        let repo = MockRepo {
            soul: Some(PromptEntry {
                name: "agent/soul.md".into(),
                description: "soul".into(),
                content: "repo soul".into(),
            }),
        };
        let result = resolve_soul(&repo, Some("settings soul")).await;
        assert_eq!(result, Some("settings soul".to_owned()));
    }

    #[tokio::test]
    async fn resolve_soul_falls_back_to_repo() {
        let repo = MockRepo {
            soul: Some(PromptEntry {
                name: "agent/soul.md".into(),
                description: "soul".into(),
                content: "repo soul".into(),
            }),
        };
        let result = resolve_soul(&repo, None).await;
        assert_eq!(result, Some("repo soul".to_owned()));
    }

    #[tokio::test]
    async fn resolve_soul_ignores_empty_settings() {
        let repo = MockRepo {
            soul: Some(PromptEntry {
                name: "agent/soul.md".into(),
                description: "soul".into(),
                content: "repo soul".into(),
            }),
        };
        let result = resolve_soul(&repo, Some("  ")).await;
        assert_eq!(result, Some("repo soul".to_owned()));
    }

    #[tokio::test]
    async fn resolve_soul_returns_none_when_empty() {
        let repo = MockRepo { soul: None };
        let result = resolve_soul(&repo, None).await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn resolve_soul_ignores_blank_repo_content() {
        let repo = MockRepo {
            soul: Some(PromptEntry {
                name: "agent/soul.md".into(),
                description: "soul".into(),
                content: "   \n  ".into(),
            }),
        };
        let result = resolve_soul(&repo, None).await;
        assert_eq!(result, None);
    }
}
