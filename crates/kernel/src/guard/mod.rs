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

//! Security guard system — taint tracking, pattern scanning, and path-scope
//! enforcement.
//!
//! Sits between permission checks and tool execution in the agent loop.
//! Three layers, checked in order (short-circuits on first block):
//! 1. **Taint tracking** — data provenance labels through the LLM context
//! 2. **Pattern scanning** — known dangerous patterns in tool arguments
//! 3. **Path-scope enforcement** — restricts file-access tools to the workspace

pub mod path_scope;
pub mod pattern;
pub mod pipeline;
pub mod taint;
