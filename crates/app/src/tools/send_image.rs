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

//! Tool for sending images to users in the current conversation.

use std::path::Path;

use async_trait::async_trait;
use rara_kernel::{
    channel::types::MessageContent,
    event::KernelEventEnvelope,
    identity::UserId,
    io::{Attachment, OutboundEnvelope},
    tool::{AgentTool, ToolContext, ToolOutput},
};
use serde_json::json;

/// Maximum file size: 10 MB (Telegram limit).
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Send an image file to the user in the current conversation.
pub struct SendImageTool;

impl SendImageTool {
    pub fn new() -> Self { Self }
}

/// Map file extension to MIME type. Returns `None` for unsupported types.
fn mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

#[async_trait]
impl AgentTool for SendImageTool {
    fn name(&self) -> &str { "send-image" }

    fn description(&self) -> &str {
        "Send an image file to the user in the current conversation. Supports PNG, JPEG, WebP, and \
         GIF formats. Maximum file size is 10 MB."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the image file on disk"
                },
                "caption": {
                    "type": "string",
                    "description": "Optional caption to accompany the image"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        // Extract required context fields.
        let session_key = context
            .session_key
            .clone()
            .ok_or_else(|| anyhow::anyhow!("send_image requires an active session context"))?;
        let origin_endpoint = context.origin_endpoint.clone();
        let event_queue = context
            .event_queue
            .clone()
            .ok_or_else(|| anyhow::anyhow!("send_image requires an active session context"))?;
        let user_id = context
            .user_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("send_image requires an active session context"))?;

        // Parse parameters.
        let file_path = params
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: file_path"))?;
        let caption = params
            .get("caption")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let path = Path::new(file_path);

        // Validate file exists.
        if !path.exists() {
            anyhow::bail!("file not found: {file_path}");
        }

        // Determine MIME type from extension.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let mime_type = mime_from_extension(ext)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unsupported image format: .{ext}. Supported: png, jpg, jpeg, webp, gif"
                )
            })?
            .to_string();

        // Validate file size.
        let metadata = std::fs::metadata(path)?;
        let file_size = metadata.len();
        if file_size > MAX_FILE_SIZE {
            anyhow::bail!(
                "file too large: {} bytes (max {} bytes / 10 MB)",
                file_size,
                MAX_FILE_SIZE
            );
        }

        // Read file bytes.
        let data = std::fs::read(path)?;

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        // Build attachment.
        let attachment = Attachment {
            data,
            mime_type: mime_type.clone(),
            filename: filename.clone(),
        };

        // Build outbound envelope.
        let content = if caption.is_empty() {
            MessageContent::Text(String::new())
        } else {
            MessageContent::Text(caption)
        };

        let envelope = OutboundEnvelope::reply(
            rara_kernel::io::MessageId::new(),
            UserId(user_id.clone()),
            session_key,
            content,
            vec![attachment],
        )
        .with_origin(origin_endpoint);

        // Push to event queue.
        event_queue
            .try_push(KernelEventEnvelope::deliver(envelope))
            .map_err(|e| anyhow::anyhow!("failed to push image event: {e}"))?;

        Ok(json!({
            "status": "sent",
            "file_path": file_path,
            "mime_type": mime_type,
            "file_size": file_size,
            "filename": filename,
        })
        .into())
    }
}
