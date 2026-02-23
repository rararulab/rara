use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum GitError {
    #[snafu(display("invalid git URL: {url}"))]
    InvalidUrl { url: String },

    #[snafu(display("clone failed: {message}"))]
    CloneFailed { message: String },

    #[snafu(display("repository not found at {path}"))]
    RepoNotFound { path: String },

    #[snafu(display("worktree error: {message}"))]
    Worktree { message: String },

    #[snafu(display("commit error: {message}"))]
    Commit { message: String },

    #[snafu(display("push error: {message}"))]
    Push { message: String },

    #[snafu(display("sync error: {message}"))]
    Sync { message: String },

    #[snafu(display("SSH key error: {message}"))]
    SshKey { message: String },

    #[snafu(display("IO error: {source}"))]
    Io { source: std::io::Error },
}
