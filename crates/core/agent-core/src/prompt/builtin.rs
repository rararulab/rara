use std::collections::HashMap;

use super::types::{PromptEntry, PromptSpec};
use super::PromptRepo;

/// Read-only prompt repository backed by compiled-in defaults.
///
/// The map is built once at construction time and never mutated,
/// so no locking is needed.
pub struct BuiltinPromptRepo {
    entries: HashMap<String, PromptEntry>,
}

impl BuiltinPromptRepo {
    /// Build a new repository from the given prompt specifications.
    #[must_use]
    pub fn new(specs: Vec<PromptSpec>) -> Self {
        let mut entries = HashMap::with_capacity(specs.len());
        for spec in specs {
            entries.insert(
                spec.name.to_owned(),
                PromptEntry {
                    name: spec.name.to_owned(),
                    description: spec.description.to_owned(),
                    content: spec.default_content.to_owned(),
                },
            );
        }
        Self { entries }
    }
}

#[async_trait::async_trait]
impl PromptRepo for BuiltinPromptRepo {
    async fn get(&self, name: &str) -> Option<PromptEntry> {
        self.entries.get(name).cloned()
    }

    async fn list(&self) -> Vec<PromptEntry> {
        self.entries.values().cloned().collect()
    }
}

/// Returns all built-in prompt specifications with their compiled-in defaults.
///
/// This is the **single source of truth** for prompt registration. Every
/// `include_str!()` referencing prompts should live here and nowhere else.
#[must_use]
pub fn all_builtin_prompts() -> Vec<PromptSpec> {
    vec![
        PromptSpec {
            name: "agent/soul.md",
            description: "Global personality / soul prompt",
            default_content: include_str!("defaults/agent/soul.md"),
        },
        PromptSpec {
            name: "chat/default_system.md",
            description: "Default chat system prompt",
            default_content: include_str!("defaults/chat/default_system.md"),
        },
        PromptSpec {
            name: "workers/agent_policy.md",
            description: "Proactive/scheduled agent operating policy",
            default_content: include_str!("defaults/workers/agent_policy.md"),
        },
        PromptSpec {
            name: "workers/resume_analysis_instructions.md",
            description: "Resume analysis tool instructions",
            default_content: include_str!(
                "defaults/workers/resume_analysis_instructions.md"
            ),
        },
        PromptSpec {
            name: "ai/cover_letter.system.md",
            description: "Cover letter agent system prompt",
            default_content: include_str!("defaults/ai/cover_letter.system.md"),
        },
        PromptSpec {
            name: "ai/follow_up.system.md",
            description: "Follow-up email agent system prompt",
            default_content: include_str!("defaults/ai/follow_up.system.md"),
        },
        PromptSpec {
            name: "ai/interview_prep.system.md",
            description: "Interview prep agent system prompt",
            default_content: include_str!("defaults/ai/interview_prep.system.md"),
        },
        PromptSpec {
            name: "ai/jd_analyzer.system.md",
            description: "Job description analyzer system prompt",
            default_content: include_str!("defaults/ai/jd_analyzer.system.md"),
        },
        PromptSpec {
            name: "ai/jd_parser.system.md",
            description: "Job description parser system prompt",
            default_content: include_str!("defaults/ai/jd_parser.system.md"),
        },
        PromptSpec {
            name: "ai/job_fit.system.md",
            description: "Job fit agent system prompt",
            default_content: include_str!("defaults/ai/job_fit.system.md"),
        },
        PromptSpec {
            name: "ai/resume_analyzer.system.md",
            description: "Resume analyzer system prompt",
            default_content: include_str!("defaults/ai/resume_analyzer.system.md"),
        },
        PromptSpec {
            name: "ai/resume_optimizer.system.md",
            description: "Resume optimizer system prompt",
            default_content: include_str!("defaults/ai/resume_optimizer.system.md"),
        },
        PromptSpec {
            name: "pipeline/pipeline.md",
            description: "Job pipeline agent system prompt",
            default_content: include_str!("defaults/pipeline/pipeline.md"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_specs() -> Vec<PromptSpec> {
        vec![
            PromptSpec {
                name: "test/hello.md",
                description: "Test prompt",
                default_content: "Hello, world!",
            },
            PromptSpec {
                name: "test/nested/deep.md",
                description: "Nested prompt",
                default_content: "Deep content",
            },
        ]
    }

    #[tokio::test]
    async fn get_returns_builtin_content() {
        let repo = BuiltinPromptRepo::new(test_specs());

        let entry = repo.get("test/hello.md").await.unwrap();
        assert_eq!(entry.content, "Hello, world!");
        assert_eq!(entry.description, "Test prompt");
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown() {
        let repo = BuiltinPromptRepo::new(test_specs());
        assert!(repo.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn list_returns_all_entries() {
        let repo = BuiltinPromptRepo::new(test_specs());
        let entries = repo.list().await;
        assert_eq!(entries.len(), 2);
    }
}
