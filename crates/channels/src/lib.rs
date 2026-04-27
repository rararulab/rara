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

//! Channel adapter implementations for the rara platform.
//!
//! This crate provides concrete
//! [`ChannelAdapter`](rara_kernel::channel::adapter::ChannelAdapter)
//! implementations for different communication platforms.
//!
//! ## Available adapters
//!
//! - [`TelegramAdapter`](telegram::TelegramAdapter) — Telegram Bot API via
//!   `getUpdates` long polling.
//! - [`WebAdapter`](web::WebAdapter) — persistent per-session WebSocket for the
//!   web chat UI (see [`web_session`]).

pub mod telegram;
pub mod terminal;
pub mod web;
pub mod web_reply_buffer;
pub mod web_session;
pub mod wechat;

/// Tool display formatting helpers.
///
/// Re-exported from [`rara_kernel::trace::tool_display`] for backward
/// compatibility with in-tree callers (`rara_channels::tool_display::...`).
/// The canonical home is the kernel because these helpers render data that
/// is persisted in [`rara_kernel::trace::ExecutionTrace`], making them a
/// trace-layer concern rather than a channel-presentation one.
pub use rara_kernel::trace::tool_display;
pub use wechat::WechatAdapter;
