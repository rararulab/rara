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

//! # rara-sessions
//!
//! Session metadata persistence layer.
//!
//! This crate provides:
//! - **[`FileSessionIndex`](file_index::FileSessionIndex)** — file-based
//!   session metadata index (tape-centric replacement for SQL-based storage).
//! - **[`types`]** — Re-exported session and message types from `rara-kernel`.

pub mod error;
pub mod file_index;
pub mod types;
