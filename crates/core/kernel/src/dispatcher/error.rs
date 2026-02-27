use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum DispatcherError {
    #[snafu(display("dispatcher channel closed"))]
    ChannelClosed,
    #[snafu(display("task not found: {task_id}"))]
    TaskNotFound { task_id: String },
    #[snafu(display("agent execution failed: {message}"))]
    AgentError { message: String },
}
