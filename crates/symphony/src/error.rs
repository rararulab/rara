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

use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SymphonyError {
    #[snafu(display("github request failed for {repo}"))]
    GitHubRequest {
        repo:     String,
        source:   reqwest::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("GitHub API returned {status} for {repo}"))]
    GitHubStatus {
        repo:     String,
        status:   reqwest::StatusCode,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("linear API error: {message}: {source}"))]
    Linear {
        message:  String,
        source:   lineark_sdk::LinearError,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("config error: {message}"))]
    Config {
        message:  String,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("config error: {message}: {source}"))]
    ConfigYaml {
        message:  String,
        source:   serde_yaml::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("git error: {source}"))]
    Git {
        source:   git2::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("workspace error: {message}"))]
    Workspace {
        message:  String,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("workspace error: {message}: {source}"))]
    WorkspaceContext {
        message:  String,
        #[snafu(source(from(SymphonyError, Box::new)))]
        source:   Box<SymphonyError>,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("workspace IO error: {message}: {source}"))]
    WorkspaceIo {
        message:  String,
        source:   std::io::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("hook failed: {hook} - {message}"))]
    Hook {
        hook:     String,
        message:  String,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("IO error: {source}"))]
    Io {
        source:   std::io::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("RPC error: {message}"))]
    Rpc {
        message:  String,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    #[snafu(display("RPC IO error: {message}: {source}"))]
    RpcIo {
        message:  String,
        source:   std::io::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },
}

pub type Result<T, E = SymphonyError> = std::result::Result<T, E>;
