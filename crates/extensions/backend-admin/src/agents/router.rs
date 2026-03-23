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

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use rara_kernel::{agent::AgentManifest, handle::KernelHandle};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AgentResponse {
    pub name:           String,
    pub description:    String,
    pub model:          Option<String>,
    pub role:           Option<String>,
    pub provider_hint:  Option<String>,
    pub max_iterations: Option<usize>,
    pub tools:          Vec<String>,
    pub builtin:        bool,
}

impl AgentResponse {
    fn from_manifest(m: &AgentManifest, builtin: bool) -> Self {
        Self {
            name: m.name.clone(),
            description: m.description.clone(),
            model: m.model.clone(),
            role: Some(format!("{:?}", m.role)),
            provider_hint: m.provider_hint.clone(),
            max_iterations: m.max_iterations,
            tools: m.tools.clone(),
            builtin,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name:           String,
    pub role:           rara_kernel::identity::Role,
    pub description:    String,
    pub model:          String,
    pub system_prompt:  String,
    #[serde(default)]
    pub soul_prompt:    Option<String>,
    #[serde(default)]
    pub provider_hint:  Option<String>,
    #[serde(default)]
    pub max_iterations: Option<usize>,
    #[serde(default)]
    pub tools:          Vec<String>,
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

enum AgentError {
    NotFound(String),
    Conflict(String),
    Internal(String),
}

impl IntoResponse for AgentError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
            Self::Conflict(msg) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
            Self::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
        }
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn agent_routes(handle: KernelHandle) -> Router {
    Router::new()
        .route("/api/v1/agents", get(list_agents).post(create_agent))
        .route("/api/v1/agents/{name}", get(get_agent).delete(delete_agent))
        .with_state(handle)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_agents(State(handle): State<KernelHandle>) -> Json<Vec<AgentResponse>> {
    let registry = handle.agent_registry();
    let agents = registry
        .list()
        .into_iter()
        .map(|m| {
            let builtin = registry.is_builtin(&m.name);
            AgentResponse::from_manifest(&m, builtin)
        })
        .collect();
    Json(agents)
}

async fn get_agent(
    State(handle): State<KernelHandle>,
    Path(name): Path<String>,
) -> Result<Json<AgentResponse>, AgentError> {
    let registry = handle.agent_registry();
    let manifest = registry
        .get(&name)
        .ok_or_else(|| AgentError::NotFound(format!("agent not found: {name}")))?;
    let builtin = registry.is_builtin(&name);
    Ok(Json(AgentResponse::from_manifest(&manifest, builtin)))
}

async fn create_agent(
    State(handle): State<KernelHandle>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<AgentResponse>), AgentError> {
    let registry = handle.agent_registry();

    if registry.get(&req.name).is_some() {
        return Err(AgentError::Conflict(format!(
            "agent already exists: {}",
            req.name
        )));
    }

    let manifest = AgentManifest {
        name:                   req.name,
        role:                   Default::default(),
        description:            req.description,
        model:                  Some(req.model),
        system_prompt:          req.system_prompt,
        soul_prompt:            req.soul_prompt,
        provider_hint:          req.provider_hint,
        max_iterations:         req.max_iterations,
        tools:                  req.tools,
        excluded_tools:         vec![],
        max_children:           None,
        max_context_tokens:     None,
        priority:               Default::default(),
        metadata:               Default::default(),
        sandbox:                None,
        default_execution_mode: None,
        tool_call_limit:        None,
        worker_timeout_secs:    None,
    };

    registry
        .register(manifest.clone(), req.role)
        .map_err(|e| AgentError::Internal(e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(AgentResponse::from_manifest(&manifest, false)),
    ))
}

async fn delete_agent(
    State(handle): State<KernelHandle>,
    Path(name): Path<String>,
) -> Result<StatusCode, AgentError> {
    let registry = handle.agent_registry();
    registry
        .unregister(&name)
        .map_err(|e| AgentError::Conflict(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}
