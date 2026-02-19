// Copyright 2025 Crrow
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Error types for skill operations.
//!
//! All errors use [`snafu`] for ergonomic context-based error construction.
//! [`SkillError`] covers I/O, frontmatter parsing, validation, network
//! requests, archive extraction, filesystem watching, and installation
//! failures.

use snafu::Snafu;

pub type Result<T> = std::result::Result<T, SkillError>;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum SkillError {
    #[snafu(display("failed to read skill file: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("invalid frontmatter in {path}: {source}"))]
    Frontmatter {
        path:   String,
        source: serde_yaml::Error,
    },

    #[snafu(display("missing frontmatter delimiters in {path}"))]
    MissingFrontmatter { path: String },

    #[snafu(display("invalid trigger regex '{pattern}': {source}"))]
    InvalidTrigger {
        pattern: String,
        source:  regex::Error,
    },

    #[snafu(display("{source}"))]
    SerdeJson {
        source:   serde_json::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("request error: {source}"))]
    Request { source: reqwest::Error },

    #[snafu(display("{message}"))]
    InvalidInput { message: String },

    #[snafu(display("skill '{name}' not found"))]
    NotFound { name: String },

    #[snafu(display("not allowed: {message}"))]
    NotAllowed { message: String },

    #[snafu(display("watcher error: {source}"))]
    Watcher {
        source: notify_debouncer_full::notify::Error,
    },

    #[snafu(display("archive error: {message}"))]
    Archive { message: String },

    #[snafu(display("task join error: {source}"))]
    TaskJoin { source: tokio::task::JoinError },

    #[snafu(display("install error: {message}"))]
    Install { message: String },

    #[snafu(display("database error: {source}"))]
    Sqlx { source: sqlx::Error },
}
