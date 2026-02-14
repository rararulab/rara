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

//! Standalone Telegram bot runtime for the Job Automation Platform.
//!
//! This crate provides a separate process that bridges Telegram users with
//! the main job service. It runs three concurrent loops:
//!
//! 1. **Telegram polling** — manual [`getUpdates`] long-polling loop that
//!    receives messages and dispatches them to command/message handlers.
//! 2. **Notification consumer** — dequeues messages from a `pgmq` queue and
//!    delivers them to Telegram chats.
//! 3. **Settings sync** — polls the KV store and hot-updates bot credentials
//!    without restarting.
//!
//! # Public API
//!
//! The only public types are [`BotApp`] (the process entry point) and
//! [`BotConfig`] / [`TelegramConfig`] (configuration). All internal modules
//! are `pub(crate)`.
//!
//! # Module Layout
//!
//! | Module          | Purpose                                               |
//! |-----------------|-------------------------------------------------------|
//! | [`config`]      | Environment parsing and dependency wiring              |
//! | [`app`]         | Process lifecycle, notification consumer, settings sync|
//! | [`bot`]         | Manual `getUpdates` long-polling loop                  |
//! | [`handlers`]    | Message routing, command handlers, callback queries    |
//! | [`state`]       | Shared runtime state with hot-update support           |
//! | [`outbound`]    | Message sending with Markdown formatting and chunking  |
//! | [`markdown`]    | Markdown-to-Telegram-HTML converter                    |
//! | [`command`]     | Telegram command definitions                           |
//! | [`http_client`] | Typed HTTP client for main service API calls           |

mod app;
mod bot;
mod command;
mod config;
mod handlers;
mod http_client;
mod markdown;
mod outbound;
mod state;

pub use app::BotApp;
pub use config::{BotConfig, TelegramConfig};
