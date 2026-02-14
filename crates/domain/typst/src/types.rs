//! Domain types for the Typst compilation service.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Domain models
// ---------------------------------------------------------------------------

/// A Typst project groups related `.typ` files and tracks render history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypstProject {
    pub id:          Uuid,
    pub name:        String,
    pub description: Option<String>,
    /// Path to the main `.typ` file (relative to project root), defaults to `"main.typ"`.
    pub main_file:   String,
    /// Optional link to a resume in the resume domain.
    pub resume_id:   Option<Uuid>,
    pub created_at:  Timestamp,
    pub updated_at:  Timestamp,
}

/// A single Typst source file belonging to a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypstFile {
    pub id:         Uuid,
    pub project_id: Uuid,
    /// Relative path within the project, e.g. `"main.typ"` or `"template/header.typ"`.
    pub path:       String,
    /// Full text content of the file.
    pub content:    String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A completed PDF render, stored in object storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderResult {
    pub id:             Uuid,
    pub project_id:     Uuid,
    /// S3 object key for the rendered PDF.
    pub pdf_object_key: String,
    /// SHA-256 hash of the concatenated source files (for caching).
    pub source_hash:    String,
    pub page_count:     i32,
    /// PDF file size in bytes.
    pub file_size:      i64,
    pub created_at:     Timestamp,
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Body for `POST /api/v1/typst/projects`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub name:        String,
    pub description: Option<String>,
    pub main_file:   Option<String>,
    pub resume_id:   Option<Uuid>,
}

/// Body for `POST /api/v1/typst/projects/{id}/files`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFileRequest {
    pub path:    String,
    pub content: String,
}

/// Body for `PUT /api/v1/typst/projects/{id}/files/{path}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateFileRequest {
    pub content: String,
}

/// Body for `POST /api/v1/typst/projects/{id}/compile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileRequest {
    /// Optionally override the main file for this compilation.
    pub main_file: Option<String>,
}

// ---------------------------------------------------------------------------
// DB row models (sqlx)
// ---------------------------------------------------------------------------

/// Raw database row for `typst_project`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TypstProjectRow {
    pub id:          Uuid,
    pub name:        String,
    pub description: Option<String>,
    pub main_file:   String,
    pub resume_id:   Option<Uuid>,
    pub created_at:  chrono::DateTime<chrono::Utc>,
    pub updated_at:  chrono::DateTime<chrono::Utc>,
}

/// Raw database row for `typst_file`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TypstFileRow {
    pub id:         Uuid,
    pub project_id: Uuid,
    pub path:       String,
    pub content:    String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Raw database row for `typst_render`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RenderResultRow {
    pub id:             Uuid,
    pub project_id:     Uuid,
    pub pdf_object_key: String,
    pub source_hash:    String,
    pub page_count:     i32,
    pub file_size:      i64,
    pub created_at:     chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Conversions: DB row -> domain model
// ---------------------------------------------------------------------------

use rara_domain_shared::convert::chrono_to_timestamp;

impl From<TypstProjectRow> for TypstProject {
    fn from(r: TypstProjectRow) -> Self {
        Self {
            id:          r.id,
            name:        r.name,
            description: r.description,
            main_file:   r.main_file,
            resume_id:   r.resume_id,
            created_at:  chrono_to_timestamp(r.created_at),
            updated_at:  chrono_to_timestamp(r.updated_at),
        }
    }
}

impl From<TypstFileRow> for TypstFile {
    fn from(r: TypstFileRow) -> Self {
        Self {
            id:         r.id,
            project_id: r.project_id,
            path:       r.path,
            content:    r.content,
            created_at: chrono_to_timestamp(r.created_at),
            updated_at: chrono_to_timestamp(r.updated_at),
        }
    }
}

impl From<RenderResultRow> for RenderResult {
    fn from(r: RenderResultRow) -> Self {
        Self {
            id:             r.id,
            project_id:     r.project_id,
            pdf_object_key: r.pdf_object_key,
            source_hash:    r.source_hash,
            page_count:     r.page_count,
            file_size:      r.file_size,
            created_at:     chrono_to_timestamp(r.created_at),
        }
    }
}
