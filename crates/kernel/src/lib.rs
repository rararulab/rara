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
pub mod cascade;
pub mod channel;
pub mod debug;
pub mod error;
pub mod event;
pub mod guard;
pub mod handle;
pub mod hooks;
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
pub mod testing;
pub mod tool;
pub mod trace;
pub mod user_question;

/// Tool name constants for kernel-provided tools.
///
/// External crates use these for manifest construction without
/// accessing internal tool modules or structs. Each constant is a
/// [`ToolName`](tool::ToolName) wrapped in `LazyLock` for zero-cost
/// reuse.
pub mod tool_names {
    use std::sync::LazyLock;

    use super::tool::ToolName;

    macro_rules! tool {
        ($name:ident, $val:expr) => {
            pub static $name: LazyLock<ToolName> = LazyLock::new(|| ToolName::new($val));
        };
    }

    tool!(TAPE_INFO, "tape-info");
    tool!(TAPE_SEARCH, "tape-search");
    tool!(TAPE_ANCHOR, "tape-anchor");
    tool!(TAPE_ANCHORS, "tape-anchors");
    tool!(TAPE_ENTRIES, "tape-entries");
    tool!(TAPE_BETWEEN, "tape-between");
    tool!(TAPE_CHECKOUT, "tape-checkout");
    tool!(TAPE_CHECKOUT_ROOT, "tape-checkout-root");
    tool!(CREATE_PLAN, "create-plan");
    tool!(KERNEL, "kernel");
    tool!(MEMORY, "memory");
    tool!(SCHEDULE_ONCE, "schedule-once");
    tool!(SCHEDULE_INTERVAL, "schedule-interval");
    tool!(SCHEDULE_CRON, "schedule-cron");
    tool!(SCHEDULE_REMOVE, "schedule-remove");
    tool!(SCHEDULE_LIST, "schedule-list");
    tool!(SPAWN_BACKGROUND, "spawn-background");
    tool!(CANCEL_BACKGROUND, "cancel-background");
    tool!(TASK, "task");
    tool!(FOLD_BRANCH, "fold-branch");

    // App-layer tools referenced by kernel presets
    tool!(BASH, "bash");
    tool!(READ_FILE, "read-file");
    tool!(WRITE_FILE, "write-file");
    tool!(EDIT_FILE, "edit-file");
    tool!(LIST_DIRECTORY, "list-directory");
    tool!(GREP, "grep");
    tool!(FIND_FILES, "find-files");
    tool!(WALK_DIRECTORY, "walk-directory");

    // Browser tools
    tool!(BROWSER_NAVIGATE, "browser-navigate");
    tool!(BROWSER_NAVIGATE_BACK, "browser-navigate-back");
    tool!(BROWSER_SNAPSHOT, "browser-snapshot");
    tool!(BROWSER_CLICK, "browser-click");
    tool!(BROWSER_TYPE, "browser-type");
    tool!(BROWSER_PRESS_KEY, "browser-press-key");
    tool!(BROWSER_EVALUATE, "browser-evaluate");
    tool!(BROWSER_WAIT_FOR, "browser-wait-for");
    tool!(BROWSER_TABS, "browser-tabs");
    tool!(BROWSER_CLOSE, "browser-close");

    // ACP delegation
    tool!(ACP_DELEGATE, "acp-delegate");

    // User interaction
    tool!(ASK_USER, "ask-user");
}

pub use error::{KernelError, Result};
