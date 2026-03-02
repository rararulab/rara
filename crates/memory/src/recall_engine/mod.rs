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

//! Agent-configurable recall strategy engine.
//!
//! The recall engine replaces hardcoded memory recall logic with a
//! rule-based system that agents can configure at runtime. Rules define
//! trigger conditions (keywords, events, turn frequency) and actions
//! (search, deep recall, profile injection).
//!
//! # Architecture
//!
//! ```text
//! Agent tools ──► RecallStrategyEngine ──► MemoryManager
//!   (CRUD)           (evaluate + execute)     (search/profile/recall)
//!       │                     │
//!       └── RecallRule ◄──────┘
//!            (trigger, action, inject target)
//! ```

pub mod defaults;
pub mod engine;
pub mod interpolation;
pub mod types;

pub use defaults::default_rules;
pub use engine::RecallStrategyEngine;
pub use types::{
    EventKind, InjectTarget, InjectionPayload, MatchedAction, RecallAction, RecallContext,
    RecallRule, RecallRuleUpdate, Trigger,
};
