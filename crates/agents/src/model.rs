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

//! Shared model types used across the agent runner.

use base::shared_string::SharedString;

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id:        SharedString,
    pub name:      SharedString,
    pub arguments: serde_json::Value,
}

// Re-exports from provider module for backward compatibility.
pub use crate::provider::*;
