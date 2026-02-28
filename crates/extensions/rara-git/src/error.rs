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
