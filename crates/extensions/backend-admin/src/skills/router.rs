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

//! HTTP API routes for skills management.
//!
//! All endpoints live under `/api/v1/skills` and use JSON request/response
//! bodies. The router is constructed via [`skill_routes`] and expects a
//! shared [`InMemoryRegistry`] as axum state.
//!
//! ## Route table
//!
//! | Method   | Path                     | Description         |
//! |----------|--------------------------|---------------------|
//! | `GET`    | `/api/v1/skills`         | List all skills     |
//! | `GET`    | `/api/v1/skills/{name}`  | Get a skill by name |
//! | `POST`   | `/api/v1/skills`         | Create a new skill  |
//! | `DELETE` | `/api/v1/skills/{name}`  | Delete a skill      |

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use rara_skills::registry::InMemoryRegistry;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Full skill response including the prompt body.
#[derive(Debug, Serialize)]
pub struct SkillResponse {
    pub name:          String,
    pub description:   String,
    pub allowed_tools: Vec<String>,
    pub source:        Option<String>,
    pub homepage:      Option<String>,
    pub license:       Option<String>,
    pub eligible:      bool,
    pub body:          String,
}

/// Compact skill listing without the prompt body.
#[derive(Debug, Serialize)]
pub struct SkillSummary {
    pub name:          String,
    pub description:   String,
    pub allowed_tools: Vec<String>,
    pub source:        Option<String>,
    pub homepage:      Option<String>,
    pub license:       Option<String>,
    pub eligible:      bool,
}

/// Request body for `POST /api/v1/skills`.
#[derive(Debug, Deserialize)]
pub struct CreateSkillRequest {
    pub name:          String,
    pub description:   String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub prompt:        String,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build an axum [`Router`] with all skill CRUD endpoints and the given
/// [`InMemoryRegistry`] as shared state.
pub fn skill_routes(registry: InMemoryRegistry) -> Router {
    Router::new()
        .route("/api/v1/skills", get(list_skills).post(create_skill))
        .route("/api/v1/skills/{name}", get(get_skill).delete(delete_skill))
        .with_state(registry)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/skills` -- list all registered skills.
async fn list_skills(State(registry): State<InMemoryRegistry>) -> Json<Vec<SkillSummary>> {
    let skills = registry
        .list_all()
        .iter()
        .map(|meta| {
            let elig = rara_skills::requirements::check_requirements(meta);
            let source_str = meta
                .source
                .as_ref()
                .map(|s| format!("{s:?}").to_lowercase());
            SkillSummary {
                name:          meta.name.clone(),
                description:   meta.description.clone(),
                allowed_tools: meta.allowed_tools.clone(),
                source:        source_str,
                homepage:      meta.homepage.clone(),
                license:       meta.license.clone(),
                eligible:      elig.eligible,
            }
        })
        .collect();
    Json(skills)
}

/// `GET /api/v1/skills/{name}` -- get a single skill by name, including its
/// prompt body.
async fn get_skill(
    State(registry): State<InMemoryRegistry>,
    Path(name): Path<String>,
) -> Result<Json<SkillResponse>, StatusCode> {
    let meta = registry.get(&name).ok_or(StatusCode::NOT_FOUND)?;

    // Load full content (async file read).
    let content = rara_skills::registry::load_skill_from_path(&meta.path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let elig = rara_skills::requirements::check_requirements(&meta);
    let source_str = meta
        .source
        .as_ref()
        .map(|s| format!("{s:?}").to_lowercase());

    Ok(Json(SkillResponse {
        name:          meta.name.clone(),
        description:   meta.description.clone(),
        allowed_tools: meta.allowed_tools.clone(),
        source:        source_str,
        homepage:      meta.homepage.clone(),
        license:       meta.license.clone(),
        eligible:      elig.eligible,
        body:          content.body,
    }))
}

/// `POST /api/v1/skills` -- create a new skill from a JSON body.
///
/// The skill is written as a `SKILL.md` file inside a new directory under the
/// user skills directory, then parsed and inserted into the in-memory registry.
async fn create_skill(
    State(registry): State<InMemoryRegistry>,
    Json(req): Json<CreateSkillRequest>,
) -> Result<(StatusCode, Json<SkillSummary>), StatusCode> {
    // Check if a skill with this name already exists.
    if registry.get(&req.name).is_some() {
        return Err(StatusCode::CONFLICT);
    }

    // Write skill directory + SKILL.md to disk.
    let skills_dir = rara_paths::skills_dir();
    let skill_dir = skills_dir.join(&req.name);
    std::fs::create_dir_all(&skill_dir).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let content = format_skill_md(&req.name, &req.description, &req.allowed_tools, &req.prompt);
    let skill_md_path = skill_dir.join("SKILL.md");
    std::fs::write(&skill_md_path, &content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Parse the written file and insert into the registry.
    let raw =
        std::fs::read_to_string(&skill_md_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut meta = rara_skills::parse::parse_metadata(&raw, &skill_dir)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    meta.source = Some(rara_skills::types::SkillSource::Personal);

    let elig = rara_skills::requirements::check_requirements(&meta);
    let source_str = meta
        .source
        .as_ref()
        .map(|s| format!("{s:?}").to_lowercase());
    let summary = SkillSummary {
        name:          meta.name.clone(),
        description:   meta.description.clone(),
        allowed_tools: meta.allowed_tools.clone(),
        source:        source_str,
        homepage:      meta.homepage.clone(),
        license:       meta.license.clone(),
        eligible:      elig.eligible,
    };

    registry.insert(meta);

    Ok((StatusCode::CREATED, Json(summary)))
}

/// `DELETE /api/v1/skills/{name}` -- delete a skill from the registry and
/// optionally remove its directory from disk.
async fn delete_skill(
    State(registry): State<InMemoryRegistry>,
    Path(name): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let meta = registry.get(&name).ok_or(StatusCode::NOT_FOUND)?;
    let skill_path = meta.path.clone();

    // Remove directory from disk (best-effort).
    let _ = std::fs::remove_dir_all(&skill_path);

    // Remove from in-memory registry.
    registry.remove(&name);

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Serialize skill data into the new SKILL.md frontmatter format.
fn format_skill_md(
    name: &str,
    description: &str,
    allowed_tools: &[String],
    prompt: &str,
) -> String {
    let mut md = String::from("---\n");
    md.push_str(&format!("name: {name}\n"));
    md.push_str(&format!(
        "description: \"{}\"\n",
        description.replace('"', "\\\"")
    ));
    if !allowed_tools.is_empty() {
        md.push_str("allowed-tools:\n");
        for tool in allowed_tools {
            md.push_str(&format!("  - {tool}\n"));
        }
    }
    md.push_str("---\n\n");
    md.push_str(prompt);
    md.push('\n');
    md
}
