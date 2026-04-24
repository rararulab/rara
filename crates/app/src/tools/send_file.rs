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
    channel::types::{ChannelType, MessageContent},
    event::KernelEventEnvelope,
    identity::UserId,
    io::{Attachment, OutboundEnvelope, StreamEvent},
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

        // Web sessions don't receive the OutboundEnvelope via the standard
        // adapter fanout (see `binding_to_endpoint` in kernel/src/io.rs —
        // Web returns `None` because chat_id == session_key). Emit the
        // bytes on the stream instead so the browser can render the file
        // inline next to the send-file tool call. Other channels already
        // render the attachment through their own adapter and would
        // double-render if they observed this event, so we scope it to
        // Web origins.
        let is_web_origin = matches!(
            context.origin_endpoint.as_ref().map(|e| e.channel_type),
            Some(ChannelType::Web)
        );
        if is_web_origin {
            if let Some(handle) = context.stream_handle.as_ref() {
                handle.emit(StreamEvent::Attachment {
                    tool_call_id: context.tool_call_id.clone(),
                    mime_type:    mime_type.to_string(),
                    filename:     filename.clone(),
                    data:         data.clone(),
                });
            }
        }

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
    use std::io::Write as _;

    use rara_kernel::{
        channel::types::ChannelType,
        io::{Endpoint, EndpointAddress, MessageId, StreamHub},
        queue::{ShardedEventQueue, ShardedEventQueueConfig},
        session::SessionKey,
    };

    use super::*;

    fn build_queue() -> rara_kernel::queue::ShardedQueueRef {
        std::sync::Arc::new(ShardedEventQueue::new(ShardedEventQueueConfig {
            num_shards:      0,
            shard_capacity:  1,
            global_capacity: 16,
        }))
    }

    fn build_context(
        origin: Option<Endpoint>,
        stream_handle: Option<rara_kernel::io::StreamHandle>,
    ) -> ToolContext {
        ToolContext {
            user_id: "test-user".into(),
            session_key: SessionKey::new(),
            origin_endpoint: origin,
            origin_user_id: None,
            event_queue: build_queue(),
            rara_message_id: MessageId::new(),
            context_window_tokens: 0,
            tool_registry: None,
            stream_handle,
            tool_call_id: Some("call-123".into()),
        }
    }

    fn web_endpoint() -> Endpoint {
        Endpoint {
            channel_type: ChannelType::Web,
            address:      EndpointAddress::Web {
                connection_id: "conn-1".into(),
            },
        }
    }

    fn telegram_endpoint() -> Endpoint {
        Endpoint {
            channel_type: ChannelType::Telegram,
            address:      EndpointAddress::Telegram {
                chat_id:   42,
                thread_id: None,
            },
        }
    }

    fn write_temp_png() -> tempfile::NamedTempFile {
        // A minimal PNG-ish payload — SendFileTool only inspects the extension
        // for MIME detection, not the byte signature.
        let mut f = tempfile::Builder::new()
            .suffix(".png")
            .tempfile()
            .expect("tempfile");
        f.write_all(b"\x89PNG\r\n\x1a\nFAKEDATA").expect("write");
        f
    }

    #[tokio::test]
    async fn attachment_stream_event_emitted_for_web_origin() {
        use rara_kernel::io::StreamEvent;

        let hub = StreamHub::new(16);
        let session = SessionKey::new();
        let handle = hub.open(session);
        // Subscribe to the per-stream broadcast directly. The session-level
        // bus is fed by an async bridge task whose scheduling cannot be
        // observed from a synchronous `try_recv` right after `emit()`.
        let streams = hub.subscribe_session(&session);
        assert_eq!(streams.len(), 1, "open() should have created one stream");
        let (_id, mut rx) = streams.into_iter().next().expect("one stream");

        let ctx = build_context(Some(web_endpoint()), Some(handle));
        let file = write_temp_png();
        let params = SendFileParams {
            file_path: file.path().to_string_lossy().into_owned(),
            caption:   Some("hi".into()),
        };
        SendFileTool.run(params, &ctx).await.expect("send-file ok");

        let mut saw_attachment = false;
        while let Ok(ev) = rx.try_recv() {
            if let StreamEvent::Attachment {
                tool_call_id,
                mime_type,
                filename,
                data,
            } = ev
            {
                assert_eq!(tool_call_id.as_deref(), Some("call-123"));
                assert_eq!(mime_type, "image/png");
                assert!(filename.is_some());
                assert!(!data.is_empty());
                saw_attachment = true;
                break;
            }
        }
        assert!(
            saw_attachment,
            "expected StreamEvent::Attachment for web origin"
        );
    }

    #[tokio::test]
    async fn attachment_stream_event_suppressed_for_telegram_origin() {
        use rara_kernel::io::StreamEvent;

        let hub = StreamHub::new(16);
        let session = SessionKey::new();
        let handle = hub.open(session);
        let streams = hub.subscribe_session(&session);
        let (_id, mut rx) = streams.into_iter().next().expect("one stream");

        let ctx = build_context(Some(telegram_endpoint()), Some(handle));
        let file = write_temp_png();
        let params = SendFileParams {
            file_path: file.path().to_string_lossy().into_owned(),
            caption:   None,
        };
        SendFileTool.run(params, &ctx).await.expect("send-file ok");

        while let Ok(ev) = rx.try_recv() {
            assert!(
                !matches!(ev, StreamEvent::Attachment { .. }),
                "Telegram origin must not emit StreamEvent::Attachment"
            );
        }
    }

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
