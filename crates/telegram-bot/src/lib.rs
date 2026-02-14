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

//! Telegram bot runtime crate.
//!
//! This crate provides a standalone bot process that:
//! - receives user messages via manual `getUpdates` long polling,
//! - calls main service HTTP APIs for search/JD parse flows,
//! - consumes notification tasks from `pgmq` for main service -> bot delivery.
//!
//! Internal module layout:
//! - `config`: env/config parsing and dependency assembly
//! - `app`: process lifecycle
//! - `bot`: manual getUpdates polling loop
//! - `handlers`: message and callback query handling
//! - `state`: centralized runtime state
//! - `outbound`: outbound message abstraction with formatting and chunking
//! - `markdown`: Markdown -> Telegram HTML converter
//! - `http_client`: bot -> main-service typed HTTP client

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
