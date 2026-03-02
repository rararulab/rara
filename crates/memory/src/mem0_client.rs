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

//! mem0 client backed by the execution worker gRPC protocol.
//!
//! The Python execution worker registers thin `mem0.*` capabilities that map
//! directly to mem0 SDK methods (e.g. `mem0.add`, `mem0.search`). This Rust
//! client provides the mem0 business API with strongly-typed request/response
//! models on top of the generic execution worker gRPC client.

use std::collections::BTreeMap;

use common_worker::{ExecutionWorkerClient, ExecutionWorkerClientError};
use serde::{Deserialize, Serialize};

use crate::error::MemoryResult;

#[derive(Clone)]
pub struct Mem0Client {
    worker: ExecutionWorkerClient,
}

impl Mem0Client {
    /// Create a new mem0 client that talks to the Python execution worker gRPC
    /// endpoint.
    pub fn new(worker_endpoint: String) -> Self {
        Self {
            worker: ExecutionWorkerClient::new(worker_endpoint),
        }
    }

    pub async fn from_config(&self, config: Mem0JsonObject) -> MemoryResult<()> {
        let _: ExecutorValue<Option<Mem0JsonValue>> =
            self.invoke("mem0.from_config", &config).await?;
        Ok(())
    }

    pub async fn add(&self, request: Mem0AddRequest) -> MemoryResult<Mem0AddResponse> {
        self.invoke("mem0.add", &request).await
    }

    pub async fn get_all(&self, request: Mem0GetAllRequest) -> MemoryResult<Mem0ListResponse> {
        self.invoke("mem0.get_all", &request).await
    }

    pub async fn get_by_id(&self, request: Mem0GetRequest) -> MemoryResult<Option<Mem0Memory>> {
        let wire: Mem0GetWireResponse = self.invoke("mem0.get", &request).await?;
        Ok(wire.into_option())
    }

    pub async fn search_mem0(&self, request: Mem0SearchRequest) -> MemoryResult<Mem0ListResponse> {
        self.invoke("mem0.search", &request).await
    }

    pub async fn update(&self, request: Mem0UpdateRequest) -> MemoryResult<Mem0StatusMessage> {
        self.invoke("mem0.update", &request).await
    }

    pub async fn history(
        &self,
        request: Mem0HistoryRequest,
    ) -> MemoryResult<Vec<Mem0HistoryEntry>> {
        let wrapped: ExecutorValue<Vec<Mem0HistoryEntry>> =
            self.invoke("mem0.history", &request).await?;
        Ok(wrapped.value)
    }

    pub async fn delete_by_id(
        &self,
        request: Mem0DeleteRequest,
    ) -> MemoryResult<Mem0StatusMessage> {
        self.invoke("mem0.delete", &request).await
    }

    pub async fn delete_all(
        &self,
        request: Mem0DeleteAllRequest,
    ) -> MemoryResult<Mem0StatusMessage> {
        self.invoke("mem0.delete_all", &request).await
    }

    pub async fn reset(&self) -> MemoryResult<()> {
        let _: ExecutorValue<Option<Mem0JsonValue>> =
            self.invoke("mem0.reset", &EmptyPayload {}).await?;
        Ok(())
    }

    /// Compatibility wrapper used by existing `MemoryManager` call sites.
    pub async fn add_memories(
        &self,
        messages: Vec<Mem0Message>,
        user_id: &str,
    ) -> MemoryResult<Vec<Mem0Event>> {
        let response = self
            .add(Mem0AddRequest {
                messages,
                user_id: Some(user_id.to_owned()),
                agent_id: None,
                run_id: None,
                metadata: None,
                infer: None,
                memory_type: None,
                prompt: None,
            })
            .await?;
        Ok(response.results)
    }

    /// Compatibility wrapper used by existing `MemoryManager` call sites.
    pub async fn search(
        &self,
        query: &str,
        user_id: &str,
        top_k: usize,
    ) -> MemoryResult<Vec<Mem0Memory>> {
        let response = self
            .search_mem0(Mem0SearchRequest {
                query:     query.to_owned(),
                user_id:   Some(user_id.to_owned()),
                run_id:    None,
                agent_id:  None,
                limit:     Some(top_k),
                filters:   None,
                threshold: None,
                rerank:    None,
            })
            .await?;
        Ok(response.results)
    }

    /// Compatibility wrapper used by existing `MemoryManager` call sites.
    pub async fn get(&self, id: &str) -> MemoryResult<Mem0Memory> {
        self.get_by_id(Mem0GetRequest {
            memory_id: id.to_owned(),
        })
        .await?
        .ok_or_else(|| crate::error::MemoryError::Mem0 {
            message: format!("mem0.get returned null for memory_id={id}"),
        })
    }

    /// Compatibility wrapper used by existing `MemoryManager` call sites.
    pub async fn delete(&self, id: &str) -> MemoryResult<()> {
        self.delete_by_id(Mem0DeleteRequest {
            memory_id: id.to_owned(),
        })
        .await?;
        Ok(())
    }

    async fn invoke<TReq, TResp>(&self, capability: &str, payload: &TReq) -> MemoryResult<TResp>
    where
        TReq: Serialize,
        TResp: for<'de> Deserialize<'de>,
    {
        self.worker
            .invoke_json(capability, payload)
            .await
            .map_err(|e| map_exec_error(capability, e))
    }
}

fn map_exec_error(capability: &str, err: ExecutionWorkerClientError) -> crate::error::MemoryError {
    crate::error::MemoryError::Mem0 {
        message: format!("{capability} failed via execution worker: {err}"),
    }
}

pub type Mem0JsonObject = BTreeMap<String, Mem0JsonValue>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Mem0JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Mem0JsonValue>),
    Object(BTreeMap<String, Mem0JsonValue>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mem0Message {
    pub role:    String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mem0AddRequest {
    pub messages:    Vec<Mem0Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id:     Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id:    Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id:      Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata:    Option<Mem0JsonObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub infer:       Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt:      Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mem0GetAllRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id:  Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id:   Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters:  Option<Mem0JsonObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit:    Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mem0SearchRequest {
    pub query:     String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id:   Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id:    Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id:  Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit:     Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters:   Option<Mem0JsonObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerank:    Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mem0GetRequest {
    pub memory_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mem0UpdateRequest {
    pub memory_id: String,
    pub data:      String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mem0HistoryRequest {
    pub memory_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mem0DeleteRequest {
    pub memory_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mem0DeleteAllRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id:  Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id:   Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mem0AddResponse {
    pub results:   Vec<Mem0Event>,
    #[serde(default)]
    pub relations: Option<Mem0JsonValue>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mem0ListResponse {
    pub results:   Vec<Mem0Memory>,
    #[serde(default)]
    pub relations: Option<Mem0JsonValue>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mem0StatusMessage {
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorValue<T> {
    pub value: T,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Mem0GetWireResponse {
    Direct(Mem0Memory),
    Wrapped { value: Option<Mem0Memory> },
}

impl Mem0GetWireResponse {
    fn into_option(self) -> Option<Mem0Memory> {
        match self {
            Self::Direct(memory) => Some(memory),
            Self::Wrapped { value } => value,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mem0Event {
    pub id:              String,
    pub event:           String,
    pub memory:          String,
    #[serde(default)]
    pub actor_id:        Option<String>,
    #[serde(default)]
    pub role:            Option<String>,
    #[serde(default)]
    pub previous_memory: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mem0Memory {
    pub id:         String,
    pub memory:     String,
    #[serde(default)]
    pub hash:       Option<String>,
    #[serde(default)]
    pub user_id:    Option<String>,
    #[serde(default)]
    pub agent_id:   Option<String>,
    #[serde(default)]
    pub run_id:     Option<String>,
    #[serde(default)]
    pub actor_id:   Option<String>,
    #[serde(default)]
    pub role:       Option<String>,
    #[serde(default)]
    pub score:      Option<f64>,
    #[serde(default)]
    pub metadata:   Option<Mem0JsonObject>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mem0HistoryEntry {
    pub id:         String,
    pub memory_id:  String,
    pub old_memory: Option<String>,
    pub new_memory: Option<String>,
    pub event:      String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub is_deleted: bool,
    #[serde(default)]
    pub actor_id:   Option<String>,
    #[serde(default)]
    pub role:       Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
struct EmptyPayload {}

#[cfg(test)]
mod tests {
    use testcontainers::{
        GenericImage, ImageExt,
        core::{IntoContainerPort, WaitFor, wait::HttpWaitStrategy},
        runners::AsyncRunner,
    };

    use super::*;

    #[test]
    fn search_request_serializes_limit_field() {
        let req = Mem0SearchRequest {
            query:     "rust".into(),
            user_id:   Some("u1".into()),
            run_id:    None,
            agent_id:  None,
            limit:     Some(5),
            filters:   None,
            threshold: None,
            rerank:    None,
        };

        let json = serde_json::to_value(req).expect("serialize");
        assert_eq!(
            json.get("limit").and_then(serde_json::Value::as_u64),
            Some(5)
        );
        assert!(json.get("top_k").is_none());
    }

    #[test]
    fn add_response_accepts_results_wrapper() {
        let json = serde_json::json!({
            "results": [
                {"id": "m1", "event": "ADD", "memory": "likes rust"}
            ]
        });

        let parsed: Mem0AddResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.results[0].event, "ADD");
    }

    #[test]
    fn get_wire_response_accepts_executor_wrapped_null() {
        let json = serde_json::json!({ "value": null });
        let parsed: Mem0GetWireResponse = serde_json::from_value(json).expect("deserialize");
        assert!(parsed.into_option().is_none());
    }

    #[tokio::test]
    async fn mem0_client_can_reach_python_worker_mem0_capability_via_testcontainer() {
        assert_local_ollama_reachable().await;

        let image_ref = std::env::var("RARA_PY_WORKER_TEST_IMAGE")
            .unwrap_or_else(|_| "rara-py-worker:3.11".to_owned());
        let (image_name, image_tag) = image_ref
            .split_once(':')
            .map_or((image_ref.as_str(), "latest"), |(n, t)| (n, t));

        let chroma = GenericImage::new("chromadb/chroma", "latest")
            .with_wait_for(WaitFor::http(
                HttpWaitStrategy::new("/api/v1/heartbeat")
                    .with_port(8000.tcp())
                    .with_expected_status_code(200_u16),
            ))
            .with_exposed_port(8000.tcp());
        let chroma_container: testcontainers::ContainerAsync<GenericImage> = chroma
            .start()
            .await
            .expect("failed to start chroma container");
        let chroma_port = chroma_container
            .get_host_port_ipv4(8000)
            .await
            .expect("failed to map chroma port");

        let image = GenericImage::new(image_name, image_tag)
            .with_wait_for(WaitFor::http(
                HttpWaitStrategy::new("/readyz")
                    .with_port(8080.tcp())
                    .with_expected_status_code(200_u16),
            ))
            .with_exposed_port(8080.tcp())
            .with_exposed_port(50051.tcp())
            .with_env_var("PYTHONUNBUFFERED", "1");
        let container = image
            .start()
            .await
            .expect("failed to start rara-py-worker container");

        let host = container
            .get_host()
            .await
            .expect("failed to get container host");
        let port = container
            .get_host_port_ipv4(50051)
            .await
            .expect("failed to map grpc port");
        let endpoint = format!("http://{host}:{port}");
        let client = Mem0Client::new(endpoint);

        let user_id = format!("it-{}", uuid::Uuid::new_v4());
        let collection = format!("mem0_it_{}", uuid::Uuid::new_v4().simple());

        client
            .from_config(valid_mem0_ollama_chroma_config(chroma_port, &collection))
            .await
            .expect("mem0.from_config should succeed with chroma + ollama config");

        let add_res = client
            .add(Mem0AddRequest {
                messages:    vec![Mem0Message {
                    role:    "user".to_owned(),
                    content: "I like Rust and I prefer terminal-based tools.".to_owned(),
                }],
                user_id:     Some(user_id.clone()),
                agent_id:    None,
                run_id:      None,
                metadata:    None,
                infer:       Some(false),
                memory_type: None,
                prompt:      None,
            })
            .await
            .expect("mem0.add should succeed");
        assert!(
            !add_res.results.is_empty(),
            "expected at least one added memory"
        );

        let all = client
            .get_all(Mem0GetAllRequest {
                user_id:  Some(user_id.clone()),
                run_id:   None,
                agent_id: None,
                filters:  None,
                limit:    Some(10),
            })
            .await
            .expect("mem0.get_all should succeed");
        assert!(
            all.results
                .iter()
                .any(|m| m.memory.to_lowercase().contains("rust")),
            "expected at least one memory mentioning rust, got: {:?}",
            all.results.iter().map(|m| &m.memory).collect::<Vec<_>>()
        );

        client
            .delete_all(Mem0DeleteAllRequest {
                user_id:  Some(user_id),
                run_id:   None,
                agent_id: None,
            })
            .await
            .expect("mem0.delete_all should succeed");
    }

    async fn assert_local_ollama_reachable() {
        let base = std::env::var("MEM0_TEST_OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".to_owned());
        let url = format!("{base}/api/tags");
        let resp = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .unwrap_or_else(|e| panic!("local ollama is not reachable at {base}: {e}"));
        assert!(
            resp.status().is_success(),
            "local ollama /api/tags returned unexpected status: {}",
            resp.status()
        );
    }

    fn valid_mem0_ollama_chroma_config(chroma_port: u16, collection: &str) -> Mem0JsonObject {
        let ollama_base_url = std::env::var("MEM0_TEST_OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://host.docker.internal:11434".to_owned());
        let llm_model = std::env::var("MEM0_TEST_OLLAMA_LLM_MODEL")
            .unwrap_or_else(|_| "llama3.2:latest".to_owned());
        let embed_model = std::env::var("MEM0_TEST_OLLAMA_EMBED_MODEL")
            .unwrap_or_else(|_| "nomic-embed-text:latest".to_owned());

        fn convert(value: serde_json::Value) -> Mem0JsonValue {
            match value {
                serde_json::Value::Null => Mem0JsonValue::Null,
                serde_json::Value::Bool(v) => Mem0JsonValue::Bool(v),
                serde_json::Value::Number(v) => {
                    Mem0JsonValue::Number(v.as_f64().expect("json number as f64"))
                }
                serde_json::Value::String(v) => Mem0JsonValue::String(v),
                serde_json::Value::Array(values) => {
                    Mem0JsonValue::Array(values.into_iter().map(convert).collect())
                }
                serde_json::Value::Object(map) => {
                    Mem0JsonValue::Object(map.into_iter().map(|(k, v)| (k, convert(v))).collect())
                }
            }
        }

        let value = serde_json::json!({
            "version": "v1.1",
            "vector_store": {
                "provider": "chroma",
                "config": {
                    "collection_name": collection,
                    "host": "host.docker.internal",
                    "port": chroma_port
                }
            },
            "llm": {
                "provider": "ollama",
                "config": {
                    "model": llm_model,
                    "ollama_base_url": ollama_base_url,
                    "temperature": 0.0,
                    "max_tokens": 256
                }
            },
            "embedder": {
                "provider": "ollama",
                "config": {
                    "model": embed_model,
                    "ollama_base_url": ollama_base_url
                }
            },
        });
        match convert(value) {
            Mem0JsonValue::Object(map) => map,
            _ => panic!("expected object"),
        }
    }
}
