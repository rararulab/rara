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

//! Browser subsystem — Lightpanda CDP-based headless browser for agent use.
//!
//! Provides page navigation, accessibility tree snapshots, DOM interaction,
//! and JavaScript evaluation via the Chrome DevTools Protocol.

pub mod error;
pub mod manager;
pub mod ref_map;
pub mod snapshot;

pub use error::{BrowserError, BrowserResult};
pub use manager::{BrowserConfig, BrowserManager, BrowserManagerRef, NavigateResult, TabInfo};
pub use ref_map::RefMap;
pub use snapshot::Snapshot;
