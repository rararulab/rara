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

//! Ollama model management endpoints: health, list, pull (SSE), delete, info.

use std::convert::Infallible;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use snafu::Snafu;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::settings::service::SettingsSvc;

// ── Errors ──────────────────────────────────────────────────

#[derive(Debug, Snafu)]
pub enum OllamaError {
    #[snafu(display("ollama unreachable at {url}: {message}"))]
    Unreachable { url: String, message: String },
    #[snafu(display("model not found: {name}"))]
    ModelNotFound { name: String },
    #[snafu(display("ollama API error: {message}"))]
    Api { message: String },
    #[snafu(display("ollama not configured: set provider to 'ollama' and configure base URL"))]
    NotConfigured,
}

impl IntoResponse for OllamaError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            OllamaError::Unreachable { .. } => (StatusCode::BAD_GATEWAY, self.to_string()),
            OllamaError::ModelNotFound { .. } => (StatusCode::NOT_FOUND, self.to_string()),
            OllamaError::Api { .. } => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            OllamaError::NotConfigured => (StatusCode::BAD_REQUEST, self.to_string()),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

// ── Response / request types ────────────────────────────────

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OllamaHealthResponse {
    pub healthy: bool,
    pub version: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OllamaLocalModel {
    pub name: String,
    pub size: u64,
    pub parameter_size: Option<String>,
    pub quantization_level: Option<String>,
    pub family: Option<String>,
    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OllamaModelListResponse {
    pub models: Vec<OllamaLocalModel>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PullModelRequest {
    pub name: String,
    #[serde(default)]
    pub insecure: bool,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DeleteModelRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OllamaModelInfo {
    pub name: String,
    pub model_info: serde_json::Value,
    pub template: Option<String>,
    pub system: Option<String>,
    pub parameters: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum PullProgressEvent {
    #[serde(rename = "progress")]
    Progress {
        status: String,
        completed: Option<u64>,
        total: Option<u64>,
    },
    #[serde(rename = "done")]
    Done { status: String },
    #[serde(rename = "error")]
    Error { message: String },
}

// ── Helpers ─────────────────────────────────────────────────

fn ollama_base_url(svc: &SettingsSvc) -> Result<String, OllamaError> {
    let settings = svc.current();
    let provider = settings.ai.provider.as_deref().unwrap_or("openrouter");
    if provider != "ollama" {
        return Err(OllamaError::NotConfigured);
    }
    Ok(settings
        .ai
        .ollama_base_url
        .unwrap_or_else(|| "http://localhost:11434".to_owned()))
}

// ── Handlers ────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/settings/ollama/health",
    tag = "settings",
    responses(
        (status = 200, description = "Ollama health status", body = OllamaHealthResponse),
    )
)]
async fn ollama_health(
    State(svc): State<SettingsSvc>,
) -> Result<Json<OllamaHealthResponse>, OllamaError> {
    let base_url = ollama_base_url(&svc)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| OllamaError::Api {
            message: e.to_string(),
        })?;

    // Check if Ollama is running
    let running = client.get(&base_url).send().await;
    if let Err(_e) = running {
        return Ok(Json(OllamaHealthResponse {
            healthy: false,
            version: None,
            url: base_url,
        }));
    }

    // Get version
    let version = match client
        .get(format!("{base_url}/api/version"))
        .send()
        .await
    {
        Ok(resp) => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            body.get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned())
        }
        Err(_) => None,
    };

    Ok(Json(OllamaHealthResponse {
        healthy: true,
        version,
        url: base_url,
    }))
}

#[utoipa::path(
    get,
    path = "/settings/ollama/models",
    tag = "settings",
    responses(
        (status = 200, description = "Local Ollama models", body = OllamaModelListResponse),
    )
)]
async fn ollama_list_models(
    State(svc): State<SettingsSvc>,
) -> Result<Json<OllamaModelListResponse>, OllamaError> {
    let base_url = ollama_base_url(&svc)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| OllamaError::Api {
            message: e.to_string(),
        })?;

    let resp = client
        .get(format!("{base_url}/api/tags"))
        .send()
        .await
        .map_err(|e| OllamaError::Unreachable {
            url: base_url.clone(),
            message: e.to_string(),
        })?;

    if !resp.status().is_success() {
        return Err(OllamaError::Api {
            message: format!("Ollama /api/tags returned {}", resp.status()),
        });
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| OllamaError::Api {
        message: format!("invalid JSON from /api/tags: {e}"),
    })?;

    let models = body
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|m| {
                    let name = m
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let size = m.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                    let modified_at = m
                        .get("modified_at")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let details = m.get("details");
                    let parameter_size = details
                        .and_then(|d| d.get("parameter_size"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let quantization_level = details
                        .and_then(|d| d.get("quantization_level"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let family = details
                        .and_then(|d| d.get("family"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    OllamaLocalModel {
                        name,
                        size,
                        parameter_size,
                        quantization_level,
                        family,
                        modified_at,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(OllamaModelListResponse { models }))
}

/// Pull model via SSE streaming.
///
/// Not annotated with `#[utoipa::path]` because SSE responses are not
/// representable in the generated OpenAPI schema.
async fn ollama_pull_model(
    State(svc): State<SettingsSvc>,
    Json(req): Json<PullModelRequest>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, OllamaError> {
    let base_url = ollama_base_url(&svc)?;
    let name = req.name.clone();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3600))
        .build()
        .map_err(|e| OllamaError::Api {
            message: e.to_string(),
        })?;

    let resp = client
        .post(format!("{base_url}/api/pull"))
        .json(&serde_json::json!({ "name": name, "insecure": req.insecure }))
        .send()
        .await
        .map_err(|e| OllamaError::Unreachable {
            url: base_url.clone(),
            message: e.to_string(),
        })?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(OllamaError::Api {
            message: format!("Pull request failed: {text}"),
        });
    }

    let byte_stream = resp.bytes_stream();

    let sse_stream = byte_stream
        .map(move |chunk: Result<axum::body::Bytes, reqwest::Error>| {
            let chunk = match chunk {
                Ok(bytes) => bytes,
                Err(e) => {
                    let event = PullProgressEvent::Error {
                        message: e.to_string(),
                    };
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    return Ok(Event::default().data(data));
                }
            };

            let text = String::from_utf8_lossy(&chunk);
            let mut last_event = None;

            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) {
                    let status = obj
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();

                    if status.contains("success") {
                        last_event = Some(PullProgressEvent::Done { status });
                    } else {
                        let completed = obj.get("completed").and_then(|v| v.as_u64());
                        let total = obj.get("total").and_then(|v| v.as_u64());
                        last_event = Some(PullProgressEvent::Progress {
                            status,
                            completed,
                            total,
                        });
                    }
                }
            }

            let event = last_event.unwrap_or(PullProgressEvent::Progress {
                status: "processing...".to_owned(),
                completed: None,
                total: None,
            });
            let data = serde_json::to_string(&event).unwrap_or_default();
            Ok(Event::default().data(data))
        });

    Ok(Sse::new(sse_stream).keep_alive(KeepAlive::default()))
}

#[utoipa::path(
    delete,
    path = "/settings/ollama/models",
    tag = "settings",
    request_body = DeleteModelRequest,
    responses(
        (status = 204, description = "Model deleted"),
    )
)]
async fn ollama_delete_model(
    State(svc): State<SettingsSvc>,
    Json(req): Json<DeleteModelRequest>,
) -> Result<StatusCode, OllamaError> {
    let base_url = ollama_base_url(&svc)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| OllamaError::Api {
            message: e.to_string(),
        })?;

    let resp = client
        .delete(format!("{base_url}/api/delete"))
        .json(&serde_json::json!({ "model": req.name }))
        .send()
        .await
        .map_err(|e| OllamaError::Unreachable {
            url: base_url.clone(),
            message: e.to_string(),
        })?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(OllamaError::ModelNotFound { name: req.name });
    }

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(OllamaError::Api {
            message: format!("Delete failed: {text}"),
        });
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/settings/ollama/models/{name}/info",
    tag = "settings",
    params(("name" = String, Path, description = "Model name")),
    responses(
        (status = 200, description = "Model info", body = OllamaModelInfo),
    )
)]
async fn ollama_model_info(
    State(svc): State<SettingsSvc>,
    Path(name): Path<String>,
) -> Result<Json<OllamaModelInfo>, OllamaError> {
    let base_url = ollama_base_url(&svc)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| OllamaError::Api {
            message: e.to_string(),
        })?;

    let resp = client
        .post(format!("{base_url}/api/show"))
        .json(&serde_json::json!({ "model": name }))
        .send()
        .await
        .map_err(|e| OllamaError::Unreachable {
            url: base_url.clone(),
            message: e.to_string(),
        })?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(OllamaError::ModelNotFound {
            name: name.clone(),
        });
    }

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(OllamaError::Api {
            message: format!("Show model info failed: {text}"),
        });
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| OllamaError::Api {
        message: format!("invalid JSON from /api/show: {e}"),
    })?;

    let model_info = body
        .get("model_info")
        .cloned()
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
    let template = body
        .get("template")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned());
    let system = body
        .get("system")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned());
    let parameters = body
        .get("parameters")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned());

    Ok(Json(OllamaModelInfo {
        name,
        model_info,
        template,
        system,
        parameters,
    }))
}

// ── Sub-router ──────────────────────────────────────────────

pub fn ollama_management_routes() -> OpenApiRouter<SettingsSvc> {
    OpenApiRouter::new()
        .routes(routes!(ollama_health))
        .routes(routes!(ollama_list_models))
        .routes(routes!(ollama_delete_model))
        .routes(routes!(ollama_model_info))
        .route(
            "/settings/ollama/models/pull",
            axum::routing::post(ollama_pull_model),
        )
}
