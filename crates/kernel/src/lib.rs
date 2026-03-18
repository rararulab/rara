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

//! # rara-kernel
//!
//! Unified session orchestrator — the single core crate for session lifecycle,
//! 6-component architecture, and LLM ↔ Tool execution loop.
//!
//! ## 6 Components
//!
//! | Component   | Trait            | Purpose                          |
//! |-------------|------------------|----------------------------------|
//! | LLM         | `LlmDriver`    | Chat completion requests         |
//! | Tool        | `ToolRegistry` | Tool registration + dispatch     |
//! | Memory      | (tape-based)     | Agent memory via tape subsystem   |
//! | Session     | `SessionIndex` | Session metadata persistence      |
//! | Guard       | `Guard`        | Tool approval + output moderation |
//! | Notification Bus | `NotificationBus` | Inter-component notification broadcasting |

pub mod agent;
pub mod browser;
pub mod cascade;
pub mod channel;
pub mod error;
pub mod event;
pub mod guard;
pub mod handle;
pub mod identity;
pub mod io;
pub mod kernel;
pub mod kv;
pub mod llm;
pub mod memory;
pub mod metrics;
pub mod mood;
pub mod notification;
pub mod plan;
pub mod proactive;
pub mod queue;
pub mod schedule;
pub mod security;
pub mod session;
pub(crate) mod syscall;
pub mod task_report;
pub mod tool;
pub mod trace;

/// Tool name constants for kernel-provided tools.
///
/// External crates use these for manifest construction without
/// accessing internal tool modules or structs.
pub mod tool_names {
    pub const TAPE: &str = "tape";
    pub const CREATE_PLAN: &str = "create-plan";
    pub const KERNEL: &str = "kernel";
    pub const MEMORY: &str = "memory";
    pub const SCHEDULE_ONCE: &str = "schedule-once";
    pub const SCHEDULE_INTERVAL: &str = "schedule-interval";
    pub const SCHEDULE_CRON: &str = "schedule-cron";
    pub const SCHEDULE_REMOVE: &str = "schedule-remove";
    pub const SCHEDULE_LIST: &str = "schedule-list";
    pub const SPAWN_BACKGROUND: &str = "spawn-background";
    pub const CANCEL_BACKGROUND: &str = "cancel-background";
    pub const FOLD_BRANCH: &str = "fold-branch";

    // Browser tools
    pub const BROWSER_NAVIGATE: &str = "browser-navigate";
    pub const BROWSER_NAVIGATE_BACK: &str = "browser-navigate-back";
    pub const BROWSER_SNAPSHOT: &str = "browser-snapshot";
    pub const BROWSER_CLICK: &str = "browser-click";
    pub const BROWSER_TYPE: &str = "browser-type";
    pub const BROWSER_PRESS_KEY: &str = "browser-press-key";
    pub const BROWSER_EVALUATE: &str = "browser-evaluate";
    pub const BROWSER_WAIT_FOR: &str = "browser-wait-for";
    pub const BROWSER_TABS: &str = "browser-tabs";
    pub const BROWSER_CLOSE: &str = "browser-close";
    pub const BROWSER_HOVER: &str = "browser-hover";
    pub const BROWSER_DRAG: &str = "browser-drag";
    pub const BROWSER_SELECT_OPTION: &str = "browser-select-option";
    pub const BROWSER_FILL_FORM: &str = "browser-fill-form";
    pub const BROWSER_HANDLE_DIALOG: &str = "browser-handle-dialog";
    pub const BROWSER_CONSOLE_MESSAGES: &str = "browser-console-messages";
    pub const BROWSER_NETWORK_REQUESTS: &str = "browser-network-requests";

    // ACP delegation
    pub const ACP_DELEGATE: &str = "acp-delegate";
}

pub use error::{KernelError, Result};
