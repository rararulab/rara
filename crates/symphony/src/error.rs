use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SymphonyError {
    #[snafu(display("github request failed for {repo}"))]
    GitHubRequest {
        repo: String,
        source: reqwest::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("GitHub API returned {status} for {repo}"))]
    GitHubStatus {
        repo: String,
        status: reqwest::StatusCode,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("linear API error: {message}"))]
    Linear {
        message: String,
        source: lineark_sdk::LinearError,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("ralph API error: {message}"))]
    Ralph {
        message: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("ralph API request failed: {source}"))]
    RalphRequest {
        source: reqwest::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("{message}"))]
    ParseJson {
        message: String,
        source: serde_json::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("config error: {message}"))]
    Config {
        message: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("git error: {source}"))]
    Git {
        source: git2::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("workspace error: {message}"))]
    Workspace {
        message: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("hook failed: {hook} - {message}"))]
    Hook {
        hook: String,
        message: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("IO error: {source}"))]
    Io {
        source: std::io::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },
}

pub type Result<T, E = SymphonyError> = std::result::Result<T, E>;
