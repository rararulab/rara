// Copyright 2025 Rararulab
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

//! Error types for the soul crate.

use std::path::PathBuf;

use snafu::Snafu;

/// Unified error type for soul operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum SoulError {
    /// Failed to parse YAML frontmatter or serialize soul file.
    #[snafu(display("failed to parse soul frontmatter: {source}"))]
    ParseFrontmatter { source: serde_yaml::Error },

    /// Failed to parse soul state YAML.
    #[snafu(display("failed to parse soul state: {source}"))]
    ParseState { source: serde_yaml::Error },

    /// Failed to serialize soul state YAML.
    #[snafu(display("failed to serialize soul state: {source}"))]
    SerializeState { source: serde_yaml::Error },

    /// Filesystem I/O error.
    #[snafu(display("I/O error at {}: {source}", path.display()))]
    Io {
        path:   PathBuf,
        source: std::io::Error,
    },
}

/// Convenience alias.
pub type Result<T, E = SoulError> = std::result::Result<T, E>;
