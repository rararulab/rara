use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum OrchestratorError {
    #[snafu(display("agent execution failed: {message}"))]
    AgentError { message: String },
}
