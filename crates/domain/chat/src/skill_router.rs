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

//! HTTP API routes for skills management.
//!
//! All endpoints live under `/api/v1/skills` and use JSON request/response
//! bodies. The router is constructed via [`skill_routes`] and expects a
//! shared [`SkillRegistry`] as axum state.
//!
//! ## Route table
//!
//! | Method   | Path                     | Description         |
//! |----------|--------------------------|---------------------|
//! | `GET`    | `/api/v1/skills`         | List all skills     |
//! | `GET`    | `/api/v1/skills/{name}`  | Get a skill by name |
//! | `POST`   | `/api/v1/skills`         | Create a new skill  |
//! | `PUT`    | `/api/v1/skills/{name}`  | Update a skill      |
//! | `DELETE` | `/api/v1/skills/{name}`  | Delete a skill      |

use std::sync::{Arc, RwLock};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use rara_skills::registry::SkillRegistry;
use serde::{Deserialize, Serialize};

/// Shared state type for skill endpoints.
type SkillState = Arc<RwLock<SkillRegistry>>;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Full skill response including the prompt body.
#[derive(Debug, Serialize)]
pub struct SkillResponse {
    pub name:        String,
    pub description: String,
    pub tools:       Vec<String>,
    pub trigger:     Option<String>,
    pub enabled:     bool,
    pub prompt:      String,
}

/// Compact skill listing without the prompt body.
#[derive(Debug, Serialize)]
pub struct SkillSummary {
    pub name:        String,
    pub description: String,
    pub tools:       Vec<String>,
    pub trigger:     Option<String>,
    pub enabled:     bool,
}

/// Request body for `POST /api/v1/skills`.
#[derive(Debug, Deserialize)]
pub struct CreateSkillRequest {
    pub name:        String,
    pub description: String,
    #[serde(default)]
    pub tools:       Vec<String>,
    pub trigger:     Option<String>,
    pub prompt:      String,
}

/// Request body for `PUT /api/v1/skills/{name}`.
#[derive(Debug, Deserialize)]
pub struct UpdateSkillRequest {
    pub description: Option<String>,
    pub tools:       Option<Vec<String>>,
    pub trigger:     Option<String>,
    pub prompt:      Option<String>,
    pub enabled:     Option<bool>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build an axum [`Router`] with all skill CRUD endpoints and the given
/// [`SkillRegistry`] as shared state.
pub fn skill_routes(registry: SkillState) -> Router {
    Router::new()
        .route("/api/v1/skills", get(list_skills).post(create_skill))
        .route(
            "/api/v1/skills/{name}",
            get(get_skill).put(update_skill).delete(delete_skill),
        )
        .with_state(registry)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/skills` — list all registered skills.
async fn list_skills(State(registry): State<SkillState>) -> Json<Vec<SkillSummary>> {
    let reg = registry.read().unwrap();
    let skills = reg
        .list_all()
        .iter()
        .map(|s| SkillSummary {
            name:        s.name().to_owned(),
            description: s.description().to_owned(),
            tools:       s.tools().to_vec(),
            trigger:     s.trigger_pattern().map(ToOwned::to_owned),
            enabled:     s.is_enabled(),
        })
        .collect();
    Json(skills)
}

/// `GET /api/v1/skills/{name}` — get a single skill by name, including its
/// prompt body.
async fn get_skill(
    State(registry): State<SkillState>,
    Path(name): Path<String>,
) -> Result<Json<SkillResponse>, StatusCode> {
    let reg = registry.read().unwrap();
    let skill = reg.get(&name).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(SkillResponse {
        name:        skill.name().to_owned(),
        description: skill.description().to_owned(),
        tools:       skill.tools().to_vec(),
        trigger:     skill.trigger_pattern().map(ToOwned::to_owned),
        enabled:     skill.is_enabled(),
        prompt:      skill.prompt.clone(),
    }))
}

/// `POST /api/v1/skills` — create a new skill from a JSON body.
///
/// The skill file is persisted to the user skills directory and inserted into
/// the in-memory registry.
async fn create_skill(
    State(registry): State<SkillState>,
    Json(req): Json<CreateSkillRequest>,
) -> Result<(StatusCode, Json<SkillResponse>), StatusCode> {
    // Check if a skill with this name already exists.
    {
        let reg = registry.read().unwrap();
        if reg.get(&req.name).is_some() {
            return Err(StatusCode::CONFLICT);
        }
    }

    // Write skill file to disk.
    let skills_dir = rara_paths::skills_dir();
    std::fs::create_dir_all(skills_dir).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let content = format_skill_md(
        &req.name,
        &req.description,
        &req.tools,
        req.trigger.as_deref(),
        true,
        &req.prompt,
    );
    let path = skills_dir.join(format!("{}.md", req.name));
    std::fs::write(&path, &content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Parse the written file and insert into the registry.
    let skill =
        rara_skills::loader::parse_skill_file(&path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = SkillResponse {
        name:        skill.name().to_owned(),
        description: skill.description().to_owned(),
        tools:       skill.tools().to_vec(),
        trigger:     skill.trigger_pattern().map(ToOwned::to_owned),
        enabled:     skill.is_enabled(),
        prompt:      skill.prompt.clone(),
    };

    registry.write().unwrap().insert(skill);

    Ok((StatusCode::CREATED, Json(response)))
}

/// `PUT /api/v1/skills/{name}` — update an existing skill (partial update).
///
/// Fields not provided in the request body are left unchanged.
async fn update_skill(
    State(registry): State<SkillState>,
    Path(name): Path<String>,
    Json(req): Json<UpdateSkillRequest>,
) -> Result<Json<SkillResponse>, StatusCode> {
    // Read existing skill data.
    let (existing_meta, existing_prompt, source_path) = {
        let reg = registry.read().unwrap();
        let skill = reg.get(&name).ok_or(StatusCode::NOT_FOUND)?;
        (
            skill.metadata.clone(),
            skill.prompt.clone(),
            skill.source_path.clone(),
        )
    };

    // Merge provided fields with existing values.
    let description = req.description.unwrap_or(existing_meta.description);
    let tools = req.tools.unwrap_or(existing_meta.tools);
    let trigger = req.trigger.or(existing_meta.trigger);
    let enabled = req.enabled.unwrap_or(existing_meta.enabled);
    let prompt = req.prompt.unwrap_or(existing_prompt);

    // Write the updated file.
    let content = format_skill_md(&name, &description, &tools, trigger.as_deref(), enabled, &prompt);
    std::fs::write(&source_path, &content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Re-parse and update registry.
    let skill = rara_skills::loader::parse_skill_file(&source_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = SkillResponse {
        name:        skill.name().to_owned(),
        description: skill.description().to_owned(),
        tools:       skill.tools().to_vec(),
        trigger:     skill.trigger_pattern().map(ToOwned::to_owned),
        enabled:     skill.is_enabled(),
        prompt:      skill.prompt.clone(),
    };

    registry.write().unwrap().insert(skill);

    Ok(Json(response))
}

/// `DELETE /api/v1/skills/{name}` — delete a skill from disk and the registry.
async fn delete_skill(
    State(registry): State<SkillState>,
    Path(name): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let source_path = {
        let reg = registry.read().unwrap();
        let skill = reg.get(&name).ok_or(StatusCode::NOT_FOUND)?;
        skill.source_path.clone()
    };

    // Remove file from disk (best-effort).
    let _ = std::fs::remove_file(&source_path);

    // Remove from in-memory registry.
    registry.write().unwrap().remove(&name);

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Serialize skill data back into the Markdown frontmatter format expected by
/// [`rara_skills::loader::parse_skill_file`].
fn format_skill_md(
    name: &str,
    description: &str,
    tools: &[String],
    trigger: Option<&str>,
    enabled: bool,
    prompt: &str,
) -> String {
    let mut md = String::from("---\n");
    md.push_str(&format!("name: {name}\n"));
    md.push_str(&format!(
        "description: \"{}\"\n",
        description.replace('"', "\\\"")
    ));
    if !tools.is_empty() {
        md.push_str("tools:\n");
        for tool in tools {
            md.push_str(&format!("  - {tool}\n"));
        }
    }
    if let Some(trigger) = trigger {
        md.push_str(&format!(
            "trigger: \"{}\"\n",
            trigger.replace('"', "\\\"")
        ));
    }
    if !enabled {
        md.push_str("enabled: false\n");
    }
    md.push_str("---\n\n");
    md.push_str(prompt);
    md.push('\n');
    md
}
