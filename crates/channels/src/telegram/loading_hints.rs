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

//! Poetic loading hints displayed in the Telegram progress message during
//! LLM thinking phases.
//!
//! Lives in the Telegram adapter rather than `kernel::io` because it has
//! zero coupling to I/O transport — it is purely a UI concern owned by
//! one channel.

use rand::Rng;

/// Pool of poetic Chinese loading messages.
pub const HINTS: &[&str] = &[
    "稍候片刻，日出文自明。",
    "风过空庭，字句正徐来。",
    "纸白微明，未成篇章。",
    "夜退星沉，此页初醒。",
    "墨痕未定，片语已生香。",
    "云开一隙，文章将至。",
    "万籁俱寂，万字将成。",
    "且听风定，再看句成。",
];

/// Return a randomly-selected loading hint.
pub fn random_hint() -> &'static str {
    let idx = rand::rng().random_range(0..HINTS.len());
    HINTS[idx]
}
