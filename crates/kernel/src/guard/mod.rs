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

//! Security guard system — taint tracking and pattern scanning.
//!
//! Sits between permission checks and tool execution in the agent loop.
//! Two layers, checked in order (short-circuits on first block):
//! 1. **Taint tracking** — data provenance labels through the LLM context
//! 2. **Pattern scanning** — known dangerous patterns in tool arguments
//!
//! The filesystem boundary previously enforced by a third "path-scope" layer
//! has been retired (#1936); write-class tools now resolve paths through
//! `rara-app::tools::path_check::resolve_writable`, which uses
//! `tokio::fs::canonicalize` so symlinks cannot escape the workspace, and
//! `bash` runs inside a `rara-sandbox` microVM with the workspace
//! bind-mounted at `/workspace`.

pub mod pattern;
pub mod pipeline;
pub mod taint;
