use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SymphonyError {
    #[snafu(display("github API error: {message}"))]
    GitHub { message: String },

    #[snafu(display("git error: {source}"))]
    Git { source: git2::Error },

    #[snafu(display("workspace error: {message}"))]
    Workspace { message: String },

    #[snafu(display("agent error: {message}"))]
    Agent { message: String },

    #[snafu(display("hook failed: {hook} — {message}"))]
    Hook { hook: String, message: String },

    #[snafu(display("config error: {message}"))]
    Config { message: String },

    #[snafu(display("IO error: {source}"))]
    Io { source: std::io::Error },
}

pub type Result<T, E = SymphonyError> = std::result::Result<T, E>;
