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

//! Send email primitive via Gmail SMTP.

use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    message::{Attachment, MultiPart, SinglePart, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use rara_domain_shared::settings::{SettingsProvider, keys};
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendEmailParams {
    /// Recipient email address.
    to:              String,
    /// Email subject line.
    subject:         String,
    /// Email body text (plain text).
    body:            String,
    /// Optional absolute path to a PDF file to attach.
    attachment_path: Option<String>,
}

/// Layer 1 primitive: send an email via Gmail SMTP.
#[derive(ToolDef)]
#[tool(
    name = "send-email",
    description = "Send an email via Gmail SMTP. Requires Gmail address and App Password to be \
                   configured in settings, and auto_send_enabled must be true. Supports optional \
                   PDF file attachment by providing the absolute path to the file on disk.",
    bypass_interceptor,
    tier = "deferred"
)]
pub struct SendEmailTool {
    settings: std::sync::Arc<dyn SettingsProvider>,
}
impl SendEmailTool {
    pub fn new(settings: std::sync::Arc<dyn SettingsProvider>) -> Self { Self { settings } }
}

#[async_trait]
impl ToolExecute for SendEmailTool {
    type Output = Value;
    type Params = SendEmailParams;

    async fn run(&self, params: SendEmailParams, _context: &ToolContext) -> anyhow::Result<Value> {
        let auto_send = self.settings.get(keys::GMAIL_AUTO_SEND_ENABLED).await;
        if auto_send.as_deref() != Some("true") {
            return Ok(serde_json::json!({"error": "auto send is disabled"}));
        }
        let from_address = match self.settings.get(keys::GMAIL_ADDRESS).await {
            Some(addr) if !addr.is_empty() => addr,
            _ => return Ok(serde_json::json!({"error": "gmail not configured: missing address"})),
        };
        let app_password = match self.settings.get(keys::GMAIL_APP_PASSWORD).await {
            Some(pw) if !pw.is_empty() => pw,
            _ => {
                return Ok(
                    serde_json::json!({"error": "gmail not configured: missing app_password"}),
                );
            }
        };
        let from_mailbox: lettre::Address =
            from_address
                .parse()
                .map_err(|e: lettre::address::AddressError| {
                    anyhow::anyhow!("invalid from address: {e}")
                })?;
        let to_mailbox: lettre::Address =
            params
                .to
                .parse()
                .map_err(|e: lettre::address::AddressError| {
                    anyhow::anyhow!("invalid to address: {e}")
                })?;
        let from_header = lettre::message::Mailbox::new(None, from_mailbox);
        let to_header = lettre::message::Mailbox::new(None, to_mailbox);
        let message = if let Some(ref path) = params.attachment_path {
            let file_bytes = tokio::fs::read(path)
                .await
                .map_err(|e| anyhow::anyhow!("failed to read attachment '{}': {}", path, e))?;
            let filename = std::path::Path::new(path).file_name().map_or_else(
                || "attachment.pdf".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            );
            let attachment = Attachment::new(filename).body(
                file_bytes,
                ContentType::parse("application/pdf").unwrap_or(ContentType::TEXT_PLAIN),
            );
            let multipart = MultiPart::mixed()
                .singlepart(SinglePart::plain(params.body.clone()))
                .singlepart(attachment);
            lettre::Message::builder()
                .from(from_header)
                .to(to_header)
                .subject(&params.subject)
                .multipart(multipart)
                .map_err(|e| anyhow::anyhow!("failed to build email message: {e}"))?
        } else {
            lettre::Message::builder()
                .from(from_header)
                .to(to_header)
                .subject(&params.subject)
                .body(params.body.clone())
                .map_err(|e| anyhow::anyhow!("failed to build email message: {e}"))?
        };
        let creds = Credentials::new(from_address.clone(), app_password);
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay("smtp.gmail.com")
            .map_err(|e| anyhow::anyhow!("failed to create SMTP transport: {e}"))?
            .credentials(creds)
            .port(587)
            .build();
        match mailer.send(message).await {
            Ok(response) => {
                tracing::info!(from = %from_address, to = %params.to, subject = %params.subject, "email sent successfully");
                Ok(
                    serde_json::json!({"sent": true, "from": from_address, "to": params.to, "subject": params.subject, "smtp_code": response.code().to_string()}),
                )
            }
            Err(e) => {
                tracing::error!(from = %from_address, to = %params.to, error = %e, "failed to send email");
                Ok(serde_json::json!({"error": format!("smtp error: {e}")}))
            }
        }
    }
}
