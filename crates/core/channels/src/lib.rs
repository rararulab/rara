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
//! - [`WebAdapter`](web::WebAdapter) — WebSocket + SSE for web chat UI.

pub mod telegram;
pub mod web;
