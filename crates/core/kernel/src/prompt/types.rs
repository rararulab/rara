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

/// Static specification for a registered prompt.
/// Includes the compiled-in default content via `include_str!()`.
#[derive(Debug, Clone)]
pub struct PromptSpec {
    /// Unique name, e.g. `"ai/job_fit.system.md"`.
    pub name:            &'static str,
    /// Human-readable description.
    pub description:     &'static str,
    /// Default content compiled into the binary.
    pub default_content: &'static str,
}

/// A resolved prompt entry with its current effective content.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PromptEntry {
    /// Unique name, e.g. `"ai/job_fit.system.md"`.
    pub name:        String,
    /// Human-readable description.
    pub description: String,
    /// Current effective content (compiled-in default).
    pub content:     String,
}

/// Errors produced by prompt operations.
#[derive(Debug, snafu::Snafu)]
#[snafu(visibility(pub))]
pub enum PromptError {
    #[snafu(display("prompt not found: {name}"))]
    NotFound { name: String },
}
