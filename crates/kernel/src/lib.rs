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
pub mod user_question;

/// Tool name constants for kernel-provided tools.
///
/// External crates use these for manifest construction without
/// accessing internal tool modules or structs.
pub mod tool_names {
    pub const TAPE_INFO: &str = "tape-info";
    pub const TAPE_SEARCH: &str = "tape-search";
    pub const TAPE_ANCHOR: &str = "tape-anchor";
    pub const TAPE_ANCHORS: &str = "tape-anchors";
    pub const TAPE_ENTRIES: &str = "tape-entries";
    pub const TAPE_BETWEEN: &str = "tape-between";
    pub const TAPE_CHECKOUT: &str = "tape-checkout";
    pub const TAPE_CHECKOUT_ROOT: &str = "tape-checkout-root";
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
    pub const TASK: &str = "task";
    pub const FOLD_BRANCH: &str = "fold-branch";

    // App-layer tools referenced by kernel presets
    pub const BASH: &str = "bash";
    pub const READ_FILE: &str = "read-file";
    pub const WRITE_FILE: &str = "write-file";
    pub const EDIT_FILE: &str = "edit-file";
    pub const LIST_DIRECTORY: &str = "list-directory";
    pub const GREP: &str = "grep";
    pub const FIND_FILES: &str = "find-files";
    pub const WALK_DIRECTORY: &str = "walk-directory";

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

    // ACP delegation
    pub const ACP_DELEGATE: &str = "acp-delegate";

    // User interaction
    pub const ASK_USER: &str = "ask-user";
}

pub use error::{KernelError, Result};
