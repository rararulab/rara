use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum SkillError {
    #[snafu(display("failed to read skill file: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("invalid frontmatter in {path}: {source}"))]
    Frontmatter {
        path: String,
        source: serde_yaml::Error,
    },

    #[snafu(display("missing frontmatter delimiters in {path}"))]
    MissingFrontmatter { path: String },

    #[snafu(display("invalid trigger regex '{pattern}': {source}"))]
    InvalidTrigger {
        pattern: String,
        source: regex::Error,
    },
}
