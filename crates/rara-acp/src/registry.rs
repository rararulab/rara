//! Known agent commands and their resolution.

use std::collections::HashMap;

/// Identifies which external coding agent to use.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentKind {
    /// Anthropic's Claude Code agent.
    Claude,
    /// OpenAI's Codex agent.
    Codex,
    /// Google's Gemini CLI agent.
    Gemini,
    /// A user-defined agent identified by name.
    Custom(String),
}

/// Resolved command to spawn an agent subprocess.
#[derive(Debug, Clone)]
pub struct AgentCommand {
    /// Executable path or command name.
    pub program: String,
    /// Command-line arguments.
    pub args:    Vec<String>,
    /// Optional environment variables to set for the subprocess.
    pub env:     Vec<(String, String)>,
}

/// Registry of known agent commands.
///
/// Maps [`AgentKind`] variants to the shell commands that spawn their ACP
/// adapters.  Built-in agents use `npx` to auto-download adapters on first
/// use.
pub struct AgentRegistry {
    agents: HashMap<AgentKind, AgentCommand>,
}

impl AgentRegistry {
    /// Create a registry pre-populated with built-in agent commands.
    pub fn with_defaults() -> Self {
        let mut agents = HashMap::new();

        agents.insert(
            AgentKind::Claude,
            AgentCommand {
                program: "npx".into(),
                args:    vec!["-y".into(), "@zed-industries/claude-agent-acp".into()],
                env:     vec![],
            },
        );

        agents.insert(
            AgentKind::Codex,
            AgentCommand {
                program: "npx".into(),
                args:    vec!["-y".into(), "@zed-industries/codex-acp".into()],
                env:     vec![],
            },
        );

        agents.insert(
            AgentKind::Gemini,
            AgentCommand {
                program: "gemini".into(),
                args:    vec!["--acp".into()],
                env:     vec![],
            },
        );

        Self { agents }
    }

    /// Register a custom agent command, replacing any previous entry for the
    /// same kind.
    pub fn register(&mut self, kind: AgentKind, command: AgentCommand) {
        self.agents.insert(kind, command);
    }

    /// Resolve an agent kind to its spawn command, returning `None` if the
    /// kind has not been registered.
    pub fn resolve(&self, kind: &AgentKind) -> Option<&AgentCommand> { self.agents.get(kind) }
}
