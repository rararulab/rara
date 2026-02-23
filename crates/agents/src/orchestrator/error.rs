use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum OrchestratorError {
    #[snafu(display("agent execution failed: {message}"))]
    AgentError { message: String },
    #[snafu(display("MCP tool discovery failed: {message}"))]
    McpError { message: String },
}
