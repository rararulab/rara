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
//!
//! Reads Gmail credentials from runtime settings at call time,
//! checks `auto_send_enabled`, and sends via `smtp.gmail.com:587` with
//! STARTTLS + App Password authentication.

use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    message::{Attachment, MultiPart, SinglePart, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use rara_domain_shared::settings::{SettingsProvider, keys};
use rara_kernel::tool::{AgentTool, ToolOutput};
use serde_json::json;

/// Layer 1 primitive: send an email via Gmail SMTP.
pub struct SendEmailTool {
    settings: std::sync::Arc<dyn SettingsProvider>,
}

impl SendEmailTool {
    pub fn new(settings: std::sync::Arc<dyn SettingsProvider>) -> Self { Self { settings } }
}

#[async_trait]
impl AgentTool for SendEmailTool {
    fn name(&self) -> &str { "send-email" }

    fn description(&self) -> &str {
        "Send an email via Gmail SMTP. Requires Gmail address and App Password to be configured in \
         settings, and auto_send_enabled must be true. Supports optional PDF file attachment by \
         providing the absolute path to the file on disk."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient email address"
                },
                "subject": {
                    "type": "string",
                    "description": "Email subject line"
                },
                "body": {
                    "type": "string",
                    "description": "Email body text (plain text)"
                },
                "attachment_path": {
                    "type": "string",
                    "description": "Optional absolute path to a PDF file to attach"
                }
            },
            "required": ["to", "subject", "body"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let to = params
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: to"))?;

        let subject = params
            .get("subject")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: subject"))?;

        let body = params
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: body"))?;

        let attachment_path = params.get("attachment_path").and_then(|v| v.as_str());

        // Read gmail settings at call time.
        let auto_send = self.settings.get(keys::GMAIL_AUTO_SEND_ENABLED).await;
        if auto_send.as_deref() != Some("true") {
            return Ok(json!({ "error": "auto send is disabled" }).into());
        }

        let from_address = match self.settings.get(keys::GMAIL_ADDRESS).await {
            Some(addr) if !addr.is_empty() => addr,
            _ => return Ok(json!({ "error": "gmail not configured: missing address" }).into()),
        };

        let app_password = match self.settings.get(keys::GMAIL_APP_PASSWORD).await {
            Some(pw) if !pw.is_empty() => pw,
            _ => {
                return Ok(json!({ "error": "gmail not configured: missing app_password" }).into());
            }
        };

        // Parse addresses.
        let from_mailbox: lettre::Address =
            from_address
                .parse()
                .map_err(|e: lettre::address::AddressError| {
                    anyhow::anyhow!("invalid from address: {e}")
                })?;
        let to_mailbox: lettre::Address =
            to.parse().map_err(|e: lettre::address::AddressError| {
                anyhow::anyhow!("invalid to address: {e}")
            })?;

        // Build the message.
        let from_header = lettre::message::Mailbox::new(None, from_mailbox);
        let to_header = lettre::message::Mailbox::new(None, to_mailbox);

        let message = if let Some(path) = attachment_path {
            // Read attachment file.
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
                .singlepart(SinglePart::plain(body.to_owned()))
                .singlepart(attachment);

            lettre::Message::builder()
                .from(from_header)
                .to(to_header)
                .subject(subject)
                .multipart(multipart)
                .map_err(|e| anyhow::anyhow!("failed to build email message: {e}"))?
        } else {
            lettre::Message::builder()
                .from(from_header)
                .to(to_header)
                .subject(subject)
                .body(body.to_owned())
                .map_err(|e| anyhow::anyhow!("failed to build email message: {e}"))?
        };

        // Build SMTP transport.
        let creds = Credentials::new(from_address.clone(), app_password);

        let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay("smtp.gmail.com")
            .map_err(|e| anyhow::anyhow!("failed to create SMTP transport: {e}"))?
            .credentials(creds)
            .port(587)
            .build();

        // Send.
        match mailer.send(message).await {
            Ok(response) => {
                tracing::info!(
                    from = %from_address,
                    to = %to,
                    subject = %subject,
                    "email sent successfully"
                );
                Ok(json!({
                    "sent": true,
                    "from": from_address,
                    "to": to,
                    "subject": subject,
                    "smtp_code": response.code().to_string(),
                })
                .into())
            }
            Err(e) => {
                tracing::error!(
                    from = %from_address,
                    to = %to,
                    error = %e,
                    "failed to send email"
                );
                Ok(json!({
                    "error": format!("smtp error: {e}"),
                })
                .into())
            }
        }
    }
}
