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

//! Integration tests for the agent OS kernel.
//!
//! Run with:
//! ```sh
//! cargo test -p rara-kernel --test integration
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::{llm::DriverRegistryBuilder, testing::TestKernelBuilder, tool::AgentTool};

/// Simple echo tool for integration testing.
struct EchoTool;

#[async_trait]
impl AgentTool for EchoTool {
    fn name(&self) -> &str { "echo_tool" }

    fn description(&self) -> &str { "Echoes back the input as-is." }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The text to echo back"
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Ok(params)
    }
}

// ---------------------------------------------------------------------------
// Test: TestKernelBuilder smoke test (no external LLM needed)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kernel_builder_creates_kernel() {
    let registry = Arc::new(
        DriverRegistryBuilder::new("test")
            .provider_model("test", "test-model", vec![])
            .build(),
    );

    let kernel = TestKernelBuilder::new()
        .driver_registry(registry)
        .tool(Arc::new(EchoTool))
        .max_concurrency(4)
        .max_iterations(10)
        .build();

    assert_eq!(kernel.config().max_concurrency, 4);
    assert_eq!(kernel.config().default_max_iterations, 10);
    assert_eq!(kernel.tool_registry().len(), 1);
    assert!(kernel.agent_registry().get("scout").is_some());
}
