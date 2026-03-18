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
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendImageParams {
    /// Absolute path to the image file on disk.
    file_path: String,
    /// Optional caption to accompany the image.
    caption:   Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendImageResult {
    pub status:    String,
    pub file_path: String,
    pub mime_type: String,
    pub file_size: u64,
    pub filename:  Option<String>,
}

/// Send an image file to the user in the current conversation.
#[derive(ToolDef)]
#[tool(
    name = "send-image",
    description = "Send an image file to the user in the current conversation. Supports PNG, \
                   JPEG, WebP, and GIF formats. Maximum file size is 10 MB."
)]
pub struct SendImageTool;
impl SendImageTool {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ToolExecute for SendImageTool {
    type Output = SendImageResult;
    type Params = SendImageParams;

    async fn run(
        &self,
        params: SendImageParams,
        context: &ToolContext,
    ) -> anyhow::Result<SendImageResult> {
        let session_key = context.session_key.clone();
        let origin_endpoint = context.origin_endpoint.clone();
        let event_queue = context.event_queue.clone();
        let user_id = &context.user_id;
        let caption = params.caption.unwrap_or_default();
        let path = Path::new(&params.file_path);
        if !path.exists() {
            anyhow::bail!("file not found: {}", params.file_path);
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let mime_type = mime_from_extension(ext)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unsupported image format: .{ext}. Supported: png, jpg, jpeg, webp, gif"
                )
            })?
            .to_string();
        let metadata = std::fs::metadata(path)?;
        let file_size = metadata.len();
        if file_size > MAX_FILE_SIZE {
            anyhow::bail!(
                "file too large: {} bytes (max {} bytes / 10 MB)",
                file_size,
                MAX_FILE_SIZE
            );
        }
        let data = std::fs::read(path)?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());
        let attachment = Attachment {
            data,
            mime_type: mime_type.clone(),
            filename: filename.clone(),
        };
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
        event_queue
            .try_push(KernelEventEnvelope::deliver(envelope))
            .map_err(|e| anyhow::anyhow!("failed to push image event: {e}"))?;
        Ok(SendImageResult {
            status: "sent".to_owned(),
            file_path: params.file_path,
            mime_type,
            file_size,
            filename,
        })
    }
}

fn mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}
