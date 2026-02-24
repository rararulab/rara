use agent_core::prompt::PromptSpec;

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
            default_content: include_str!("../../../../../prompts/agent/soul.md"),
        },
        PromptSpec {
            name: "chat/default_system.md",
            description: "Default chat system prompt",
            default_content: include_str!("../../../../../prompts/chat/default_system.md"),
        },
        PromptSpec {
            name: "workers/agent_policy.md",
            description: "Proactive/scheduled agent operating policy",
            default_content: include_str!("../../../../../prompts/workers/agent_policy.md"),
        },
        PromptSpec {
            name: "workers/resume_analysis_instructions.md",
            description: "Resume analysis tool instructions",
            default_content: include_str!(
                "../../../../../prompts/workers/resume_analysis_instructions.md"
            ),
        },
        PromptSpec {
            name: "ai/cover_letter.system.md",
            description: "Cover letter agent system prompt",
            default_content: include_str!("../../../../../prompts/ai/cover_letter.system.md"),
        },
        PromptSpec {
            name: "ai/follow_up.system.md",
            description: "Follow-up email agent system prompt",
            default_content: include_str!("../../../../../prompts/ai/follow_up.system.md"),
        },
        PromptSpec {
            name: "ai/interview_prep.system.md",
            description: "Interview prep agent system prompt",
            default_content: include_str!("../../../../../prompts/ai/interview_prep.system.md"),
        },
        PromptSpec {
            name: "ai/jd_analyzer.system.md",
            description: "Job description analyzer system prompt",
            default_content: include_str!("../../../../../prompts/ai/jd_analyzer.system.md"),
        },
        PromptSpec {
            name: "ai/jd_parser.system.md",
            description: "Job description parser system prompt",
            default_content: include_str!("../../../../../prompts/ai/jd_parser.system.md"),
        },
        PromptSpec {
            name: "ai/job_fit.system.md",
            description: "Job fit agent system prompt",
            default_content: include_str!("../../../../../prompts/ai/job_fit.system.md"),
        },
        PromptSpec {
            name: "ai/resume_analyzer.system.md",
            description: "Resume analyzer system prompt",
            default_content: include_str!("../../../../../prompts/ai/resume_analyzer.system.md"),
        },
        PromptSpec {
            name: "ai/resume_optimizer.system.md",
            description: "Resume optimizer system prompt",
            default_content: include_str!("../../../../../prompts/ai/resume_optimizer.system.md"),
        },
        PromptSpec {
            name: "pipeline/pipeline.md",
            description: "Job pipeline agent system prompt",
            default_content: include_str!("../../../../../prompts/pipeline/pipeline.md"),
        },
    ]
}
