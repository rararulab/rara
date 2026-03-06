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

//! Telegram bot command handler implementations.
//!
//! Each module implements
//! [`CommandHandler`](rara_kernel::channel::command::CommandHandler)
//! or [`CallbackHandler`](rara_kernel::channel::command::CallbackHandler) for a
//! group of related bot commands.
//!
//! ## Modules
//!
//! - [`client`]: Backend service client trait and response types.
//! - [`basic`]: `/start` and `/help` commands.
//! - [`session`]: `/new`, `/clear`, `/sessions`, `/usage`, `/model` commands.
//! - [`job`]: `/search` and `/jd` commands.
//! - [`mcp`]: `/mcp` command.
//! - [`callbacks`]: Inline keyboard callback handlers.

pub mod basic;
pub mod callbacks;
pub mod client;
pub mod kernel_client;
pub mod mcp;
pub mod session;

pub use basic::BasicCommandHandler;
pub use callbacks::SessionSwitchCallbackHandler;
pub use client::BotServiceClient;
pub use kernel_client::KernelBotServiceClient;
pub use mcp::McpCommandHandler;
pub use session::{SessionCommandHandler, StopCommandHandler};
