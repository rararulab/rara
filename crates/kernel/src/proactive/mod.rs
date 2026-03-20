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

//! Proactive subsystem — event-driven signals + group-chat judgment.
//!
//! This module contains:
//! - **Signal** — typed proactive events emitted by the kernel
//! - **Config** — YAML-driven filter configuration
//! - **Filter** — pure rule-based gate (quiet hours, cooldowns, rate limits)
//! - **Context** — structured context pack builder for Mita
//! - **Judgment** — lightweight LLM judgment for group-chat replies

mod config;
mod context;
mod filter;
mod judgment;
mod signal;

pub use config::ProactiveConfig;
pub use context::{SessionContext, build_context_pack, build_heartbeat_context_pack};
pub use filter::ProactiveFilter;
pub use judgment::{ProactiveJudgment, should_reply};
pub use signal::ProactiveSignal;

/// Truncate a string to at most `max` characters.
pub(crate) fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}
