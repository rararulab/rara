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

//! WeChat iLink Bot channel adapter.
//!
//! Implements [`ChannelAdapter`] using the WeChat iLink Bot API via
//! long-polling `getUpdates`. Inbound messages are converted to
//! [`RawPlatformMessage`] and handed to the [`KernelHandle`]. Outbound
//! delivery converts markdown to plain text before sending.

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use dashmap::DashMap;
use rara_kernel::{
    channel::{
        adapter::ChannelAdapter,
        types::{ChannelType, MessageContent},
    },
    error::KernelError,
    handle::KernelHandle,
    io::{EgressError, Endpoint, EndpointAddress, PlatformOutbound, RawPlatformMessage},
};
use tokio::sync::{Mutex, watch};
use tracing::{error, info, instrument, warn};

use super::{
    api::WeixinApiClient,
    runtime::{body_from_item_list, markdown_to_plain_text},
    storage,
};

/// Channel adapter for WeChat iLink Bot.
///
/// Uses long-polling to receive inbound messages and the iLink API to
/// send outbound replies. Context tokens (required by the iLink protocol
/// for reply routing) are cached per user.
///
/// Two separate API clients are used: one dedicated to the long-polling
/// loop (inbound) and one for outbound sends, so that the long-poll
/// never blocks outbound delivery.
pub struct WechatAdapter {
    /// API client for outbound sends (text, media, typing).
    send_client:    WeixinApiClient,
    /// API client for inbound long-polling (held behind a Mutex so the
    /// spawned task can own it).
    poll_client:    Arc<Mutex<WeixinApiClient>>,
    /// WeChat account identifier used for credential and buffer storage.
    account_id:     String,
    /// Sender half of the shutdown signal.
    shutdown_tx:    watch::Sender<bool>,
    /// Receiver half of the shutdown signal.
    shutdown_rx:    watch::Receiver<bool>,
    /// Maps `user_id` to the latest `context_token` received from that user.
    /// The iLink API requires a context token when sending replies.
    context_tokens: Arc<DashMap<String, String>>,
    /// The bot's own WeChat user ID (ilink_user_id from login credentials).
    bot_user_id:    String,
    /// Handle to the spawned polling task for graceful shutdown.
    poll_handle:    Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl WechatAdapter {
    /// Creates a new adapter for the given WeChat account.
    ///
    /// Loads credentials from local storage and initialises two API clients
    /// (one for polling, one for sending) to avoid mutex contention.
    pub fn new(account_id: String, base_url: String) -> Result<Self, KernelError> {
        let account_data =
            storage::get_account_data(&account_id).map_err(|e| KernelError::Boot {
                message: format!("failed to load wechat account data: {e}"),
            })?;

        let route_tag = storage::get_account_config(&account_id).and_then(|c| c.route_tag);

        let send_client = WeixinApiClient::new(&base_url, &account_data.token, route_tag.clone());
        let poll_client = WeixinApiClient::new(&base_url, &account_data.token, route_tag);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        info!(
            account_id = %account_id,
            bot_user_id = %account_data.user_id,
            "wechat adapter initialized"
        );

        Ok(Self {
            send_client,
            poll_client: Arc::new(Mutex::new(poll_client)),
            account_id,
            shutdown_tx,
            shutdown_rx,
            context_tokens: Arc::new(DashMap::new()),
            bot_user_id: account_data.user_id,
            poll_handle: Mutex::new(None),
        })
    }
}

#[async_trait]
impl ChannelAdapter for WechatAdapter {
    fn channel_type(&self) -> ChannelType { ChannelType::Wechat }

    #[instrument(skip_all)]
    async fn start(&self, handle: KernelHandle) -> Result<(), KernelError> {
        let poll_client = Arc::clone(&self.poll_client);
        let account_id = self.account_id.clone();
        let bot_user_id = self.bot_user_id.clone();
        let context_tokens = Arc::clone(&self.context_tokens);
        let shutdown_rx = self.shutdown_rx.clone();

        info!(
            account_id = %account_id,
            bot_user_id = %bot_user_id,
            "starting wechat long-polling loop"
        );

        let join_handle = tokio::spawn(async move {
            let mut consecutive_errors: u32 = 0;
            let mut buf = storage::get_updates_buf(&account_id);

            loop {
                if *shutdown_rx.borrow() {
                    info!("wechat polling loop received shutdown signal");
                    break;
                }

                let result = {
                    let client = poll_client.lock().await;
                    client.get_updates(buf.as_deref()).await
                };

                match result {
                    Ok(resp) => {
                        consecutive_errors = 0;

                        if let Some(new_buf) = resp["get_updates_buf"].as_str() {
                            buf = Some(new_buf.to_string());
                            let _ = storage::save_updates_buf(&account_id, new_buf);
                        }

                        if let Some(messages) = resp["msgs"].as_array() {
                            info!(count = messages.len(), "wechat poll returned messages");
                            for msg in messages {
                                let item_list =
                                    msg["item_list"].as_array().cloned().unwrap_or_default();

                                let Some(from_user_id) = msg["from_user_id"]
                                    .as_str()
                                    .filter(|s| !s.is_empty())
                                    .map(String::from)
                                else {
                                    warn!("skipping wechat message with missing from_user_id");
                                    continue;
                                };

                                let context_token =
                                    msg["context_token"].as_str().unwrap_or("").to_string();

                                // Cache the context token for outbound replies.
                                if !context_token.is_empty() {
                                    context_tokens.insert(from_user_id.clone(), context_token);
                                }

                                let body = body_from_item_list(&item_list);
                                if body.is_empty() {
                                    info!(from_user_id, "skipping wechat message with empty body");
                                    continue;
                                }

                                info!(
                                    bot_user_id = %bot_user_id,
                                    from_user_id,
                                    body_len = body.len(),
                                    body_preview = &body[..body.len().min(100)],
                                    "received wechat message, ingesting"
                                );

                                let raw = RawPlatformMessage {
                                    channel_type:        ChannelType::Wechat,
                                    platform_message_id: None,
                                    platform_user_id:    from_user_id.clone(),
                                    platform_chat_id:    Some(from_user_id),
                                    content:             MessageContent::Text(body),
                                    reply_context:       None,
                                    metadata:            HashMap::new(),
                                };

                                if let Err(e) = handle.ingest(raw).await {
                                    error!(error = %e, "failed to ingest wechat message");
                                }
                            }
                        }
                    }
                    Err(super::errors::Error::SessionExpired) => {
                        warn!("wechat session expired, stopping polling loop");
                        break;
                    }
                    Err(super::errors::Error::Http { ref source }) if source.is_timeout() => {
                        // Long-poll timeout is normal — just retry.
                    }
                    Err(e) => {
                        consecutive_errors += 1;
                        error!(consecutive_errors, "wechat get_updates error: {e}");
                        if consecutive_errors >= 3 {
                            warn!("too many consecutive errors, backing off 30s");
                            tokio::time::sleep(Duration::from_secs(30)).await;
                            consecutive_errors = 0;
                        } else {
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                    }
                }
            }

            info!("wechat polling loop exited");
        });

        *self.poll_handle.lock().await = Some(join_handle);

        Ok(())
    }

    #[instrument(skip_all)]
    async fn send(&self, endpoint: &Endpoint, msg: PlatformOutbound) -> Result<(), EgressError> {
        let user_id = match &endpoint.address {
            EndpointAddress::Wechat { user_id } => user_id.clone(),
            other => {
                return Err(EgressError::DeliveryFailed {
                    message: format!("expected Wechat endpoint, got: {other:?}"),
                });
            }
        };

        let context_token = self.context_tokens.get(&user_id).map(|r| r.value().clone());

        match msg {
            PlatformOutbound::Reply { content, .. } => {
                let token = context_token.ok_or_else(|| EgressError::DeliveryFailed {
                    message: format!(
                        "no context_token cached for wechat user {user_id} — cannot send without \
                         a prior inbound message"
                    ),
                })?;
                let plain = markdown_to_plain_text(&content);
                // Send to the bot's own account_id — the iLink API uses the
                // context_token (not to_user_id) to route the reply to the
                // actual human user.
                info!(
                    to = %self.account_id,
                    text_len = plain.len(),
                    "sending wechat reply"
                );
                let result = self
                    .send_client
                    .send_text_message(&self.account_id, &token, &plain)
                    .await;
                info!(?result, "wechat send_text_message result");
                result.map_err(|e| EgressError::DeliveryFailed {
                    message: format!("wechat send_text_message failed: {e}"),
                })?;
            }
            // WeChat does not support streaming edits or progress messages.
            PlatformOutbound::StreamChunk { .. } | PlatformOutbound::Progress { .. } => {}
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn stop(&self) -> Result<(), KernelError> {
        info!("stopping wechat adapter");
        let _ = self.shutdown_tx.send(true);

        // Await the polling task so it shuts down cleanly.
        let handle = self.poll_handle.lock().await.take();
        if let Some(handle) = handle {
            if let Err(e) = handle.await {
                warn!(error = %e, "wechat polling task panicked during shutdown");
            }
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn typing_indicator(&self, session_key: &str) -> Result<(), KernelError> {
        let context_token = self
            .context_tokens
            .get(session_key)
            .map(|r| r.value().clone())
            .unwrap_or_default();

        // Best-effort — typing indicators are optional UX hooks.
        // Send to account_id; context_token routes to the human user.
        let _ = self
            .send_client
            .send_typing(&self.account_id, &context_token)
            .await;
        Ok(())
    }
}
