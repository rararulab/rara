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

//! Tool for sending files (images, documents, archives, etc.) to users.
//!
//! Replaces the old `send-image` tool which was artificially limited to
//! four image formats. The underlying `Attachment` + adapter layer already
//! supports arbitrary files: images are sent as photos, everything else
//! as documents.

use std::path::Path;

use async_trait::async_trait;
use rara_kernel::{
    channel::types::MessageContent,
    event::KernelEventEnvelope,
    identity::UserId,
    io::{Attachment, OutboundEnvelope},
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50 MB

/// Input parameters for the send-file tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendFileParams {
    /// Absolute path to the file on disk.
    file_path: String,
    /// Optional caption or message to accompany the file.
    caption:   Option<String>,
}

/// Typed result returned by the send-file tool.
#[derive(Debug, Clone, Serialize)]
pub struct SendFileResult {
    pub status:    String,
    pub file_path: String,
    pub mime_type: String,
    pub file_size: u64,
    pub filename:  Option<String>,
}

/// Send a file to the user in the current conversation.
///
/// Images (PNG, JPEG, WebP, GIF) are delivered as photos; all other
/// formats are sent as documents. Maximum 50 MB.
#[derive(ToolDef)]
#[tool(
    name = "send-file",
    description = "Send a file to the user. Images (png/jpg/webp/gif) are sent as photos; other \
                   formats (pdf, csv, zip, etc.) are sent as documents. Max 50 MB.",
    tier = "deferred"
)]
pub struct SendFileTool;

impl SendFileTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for SendFileTool {
    type Output = SendFileResult;
    type Params = SendFileParams;

    async fn run(
        &self,
        params: SendFileParams,
        context: &ToolContext,
    ) -> anyhow::Result<SendFileResult> {
        let path = Path::new(&params.file_path);
        if !path.exists() {
            anyhow::bail!("file not found: {}", params.file_path);
        }

        let metadata = std::fs::metadata(path)?;
        if metadata.len() > MAX_FILE_SIZE {
            anyhow::bail!(
                "file too large: {} bytes (max {} MB)",
                metadata.len(),
                MAX_FILE_SIZE / (1024 * 1024)
            );
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mime_type = mime_from_extension(&ext);

        let data = std::fs::read(path)?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        let attachment = Attachment {
            data,
            mime_type: mime_type.to_string(),
            filename: filename.clone(),
        };

        let caption = params.caption.unwrap_or_default();
        let content = if caption.is_empty() {
            MessageContent::Text(String::new())
        } else {
            MessageContent::Text(caption)
        };

        let envelope = OutboundEnvelope::reply(
            rara_kernel::io::MessageId::new(),
            UserId(context.user_id.clone()),
            context.session_key.clone(),
            content,
            vec![attachment],
        )
        .with_origin(context.origin_endpoint.clone());

        context
            .event_queue
            .clone()
            .try_push(KernelEventEnvelope::deliver(envelope))
            .map_err(|e| anyhow::anyhow!("failed to push file event: {e}"))?;

        Ok(SendFileResult {
            status: "sent".to_owned(),
            file_path: params.file_path,
            mime_type: mime_type.to_string(),
            file_size: metadata.len(),
            filename,
        })
    }
}

/// Best-effort MIME type from file extension. Falls back to
/// `application/octet-stream` for unknown types — the adapter layer
/// uses the MIME prefix (`image/*` → photo, else → document).
fn mime_from_extension(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "csv" => "text/csv",
        "json" => "application/json",
        "txt" | "log" | "md" => "text/plain",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" | "tgz" => "application/gzip",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "doc" | "docx" => "application/msword",
        "xls" | "xlsx" => "application/vnd.ms-excel",
        "ppt" | "pptx" => "application/vnd.ms-powerpoint",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_images_detected() {
        assert_eq!(mime_from_extension("png"), "image/png");
        assert_eq!(mime_from_extension("jpg"), "image/jpeg");
        assert_eq!(mime_from_extension("jpeg"), "image/jpeg");
        assert_eq!(mime_from_extension("webp"), "image/webp");
        assert_eq!(mime_from_extension("gif"), "image/gif");
    }

    #[test]
    fn mime_documents_detected() {
        assert_eq!(mime_from_extension("pdf"), "application/pdf");
        assert_eq!(mime_from_extension("csv"), "text/csv");
        assert_eq!(mime_from_extension("json"), "application/json");
        assert_eq!(mime_from_extension("txt"), "text/plain");
        assert_eq!(mime_from_extension("md"), "text/plain");
        assert_eq!(mime_from_extension("zip"), "application/zip");
    }

    #[test]
    fn mime_unknown_falls_back() {
        assert_eq!(mime_from_extension("xyz"), "application/octet-stream");
        assert_eq!(mime_from_extension(""), "application/octet-stream");
        assert_eq!(mime_from_extension("rara"), "application/octet-stream");
    }

    #[test]
    fn images_have_image_prefix() {
        for ext in &["png", "jpg", "jpeg", "webp", "gif", "svg"] {
            assert!(
                mime_from_extension(ext).starts_with("image/"),
                "{ext} should be image/*"
            );
        }
    }

    #[test]
    fn non_images_lack_image_prefix() {
        for ext in &["pdf", "csv", "zip", "mp3", "docx", "xyz"] {
            assert!(
                !mime_from_extension(ext).starts_with("image/"),
                "{ext} should not be image/*"
            );
        }
    }
}
