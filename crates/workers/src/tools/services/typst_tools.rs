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

//! Layer 2 service tools for Typst project management and compilation.
//!
//! - [`ListTypstProjectsTool`]: list all Typst projects.
//! - [`ListTypstFilesTool`]: list files in a project.
//! - [`ReadTypstFileTool`]: read file content from a project.
//! - [`UpdateTypstFileTool`]: update file content in a project.
//! - [`CompileTypstProjectTool`]: compile a project to PDF.

use async_trait::async_trait;
use rara_agents::tool_registry::AgentTool;
use serde_json::json;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// list_typst_projects
// ---------------------------------------------------------------------------

/// Layer 2 service tool: list all Typst projects.
pub struct ListTypstProjectsTool {
    typst_service: rara_domain_typst::service::TypstService,
}

impl ListTypstProjectsTool {
    pub fn new(typst_service: rara_domain_typst::service::TypstService) -> Self {
        Self { typst_service }
    }
}

#[async_trait]
impl AgentTool for ListTypstProjectsTool {
    fn name(&self) -> &str { "list_typst_projects" }

    fn description(&self) -> &str {
        "List all Typst projects. Returns project id, name, main file, and git URL if imported from Git."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _params: serde_json::Value) -> rara_agents::err::Result<serde_json::Value> {
        match self.typst_service.list_projects().await {
            Ok(projects) => {
                let items: Vec<serde_json::Value> = projects
                    .iter()
                    .map(|p| {
                        json!({
                            "id": p.id.to_string(),
                            "name": p.name,
                            "main_file": p.main_file,
                            "git_url": p.git_url,
                        })
                    })
                    .collect();
                Ok(json!({ "projects": items, "count": items.len() }))
            }
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}

// ---------------------------------------------------------------------------
// list_typst_files
// ---------------------------------------------------------------------------

/// Layer 2 service tool: list all files in a Typst project.
pub struct ListTypstFilesTool {
    typst_service: rara_domain_typst::service::TypstService,
}

impl ListTypstFilesTool {
    pub fn new(typst_service: rara_domain_typst::service::TypstService) -> Self {
        Self { typst_service }
    }
}

#[async_trait]
impl AgentTool for ListTypstFilesTool {
    fn name(&self) -> &str { "list_typst_files" }

    fn description(&self) -> &str {
        "List all files in a Typst project."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "project_id": {
                    "type": "string",
                    "description": "The UUID of the Typst project"
                }
            },
            "required": ["project_id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> rara_agents::err::Result<serde_json::Value> {
        let project_id = parse_uuid(&params, "project_id")?;

        match self.typst_service.list_files(project_id).await {
            Ok(files) => {
                let items: Vec<serde_json::Value> = files
                    .iter()
                    .map(|f| json!({ "path": f.path }))
                    .collect();
                Ok(json!({ "files": items, "count": items.len() }))
            }
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}

// ---------------------------------------------------------------------------
// read_typst_file
// ---------------------------------------------------------------------------

/// Layer 2 service tool: read the content of a file in a Typst project.
pub struct ReadTypstFileTool {
    typst_service: rara_domain_typst::service::TypstService,
}

impl ReadTypstFileTool {
    pub fn new(typst_service: rara_domain_typst::service::TypstService) -> Self {
        Self { typst_service }
    }
}

#[async_trait]
impl AgentTool for ReadTypstFileTool {
    fn name(&self) -> &str { "read_typst_file" }

    fn description(&self) -> &str {
        "Read the content of a file in a Typst project."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "project_id": {
                    "type": "string",
                    "description": "The UUID of the Typst project"
                },
                "file_path": {
                    "type": "string",
                    "description": "The relative file path within the project (e.g. \"main.typ\")"
                }
            },
            "required": ["project_id", "file_path"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> rara_agents::err::Result<serde_json::Value> {
        let project_id = parse_uuid(&params, "project_id")?;
        let file_path = parse_string(&params, "file_path")?;

        match self.typst_service.get_file(project_id, &file_path).await {
            Ok(file) => Ok(json!({
                "project_id": project_id.to_string(),
                "path": file.path,
                "content": file.content,
            })),
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}

// ---------------------------------------------------------------------------
// update_typst_file
// ---------------------------------------------------------------------------

/// Layer 2 service tool: update the content of a file in a Typst project.
pub struct UpdateTypstFileTool {
    typst_service: rara_domain_typst::service::TypstService,
}

impl UpdateTypstFileTool {
    pub fn new(typst_service: rara_domain_typst::service::TypstService) -> Self {
        Self { typst_service }
    }
}

#[async_trait]
impl AgentTool for UpdateTypstFileTool {
    fn name(&self) -> &str { "update_typst_file" }

    fn description(&self) -> &str {
        "Update the content of a file in a Typst project. Use this to modify resume content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "project_id": {
                    "type": "string",
                    "description": "The UUID of the Typst project"
                },
                "file_path": {
                    "type": "string",
                    "description": "The relative file path within the project (e.g. \"main.typ\")"
                },
                "content": {
                    "type": "string",
                    "description": "The new content for the file"
                }
            },
            "required": ["project_id", "file_path", "content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> rara_agents::err::Result<serde_json::Value> {
        let project_id = parse_uuid(&params, "project_id")?;
        let file_path = parse_string(&params, "file_path")?;
        let content = parse_string(&params, "content")?;

        match self.typst_service.update_file(project_id, &file_path, content).await {
            Ok(file) => Ok(json!({
                "project_id": project_id.to_string(),
                "path": file.path,
                "updated_at": file.updated_at.to_string(),
                "message": "file updated successfully",
            })),
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}

// ---------------------------------------------------------------------------
// compile_typst_project
// ---------------------------------------------------------------------------

/// Layer 2 service tool: compile a Typst project to PDF.
pub struct CompileTypstProjectTool {
    typst_service: rara_domain_typst::service::TypstService,
}

impl CompileTypstProjectTool {
    pub fn new(typst_service: rara_domain_typst::service::TypstService) -> Self {
        Self { typst_service }
    }
}

#[async_trait]
impl AgentTool for CompileTypstProjectTool {
    fn name(&self) -> &str { "compile_typst_project" }

    fn description(&self) -> &str {
        "Compile a Typst project to PDF. Returns compilation result including page count and file size."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "project_id": {
                    "type": "string",
                    "description": "The UUID of the Typst project to compile"
                }
            },
            "required": ["project_id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> rara_agents::err::Result<serde_json::Value> {
        let project_id = parse_uuid(&params, "project_id")?;

        match self.typst_service.compile(project_id, None).await {
            Ok(render) => Ok(json!({
                "project_id": project_id.to_string(),
                "render_id": render.id.to_string(),
                "page_count": render.page_count,
                "file_size": render.file_size,
                "source_hash": render.source_hash,
                "message": "compilation successful",
            })),
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a required UUID parameter from JSON.
fn parse_uuid(params: &serde_json::Value, field: &str) -> rara_agents::err::Result<Uuid> {
    let s = params
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| rara_agents::err::Error::Other {
            message: format!("missing required parameter: {field}").into(),
        })?;

    Uuid::parse_str(s).map_err(|e| rara_agents::err::Error::Other {
        message: format!("invalid UUID for {field}: {e}").into(),
    })
}

/// Extract a required string parameter from JSON.
fn parse_string(params: &serde_json::Value, field: &str) -> rara_agents::err::Result<String> {
    params
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned())
        .ok_or_else(|| rara_agents::err::Error::Other {
            message: format!("missing required parameter: {field}").into(),
        })
}
