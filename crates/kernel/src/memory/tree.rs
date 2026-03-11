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

//! Anchor tree types used for visualizing forked sessions.

use serde::{Deserialize, Serialize};

/// A node in the anchor tree representing one anchor in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnchorNode {
    /// Anchor name (e.g. `session/start`, `topic/weather`).
    pub name:     String,
    /// Optional summary from handoff state.
    pub summary:  Option<String>,
    /// Tape entry id of the anchor.
    pub entry_id: u64,
}

/// A fork edge from one anchor into a child session branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkEdge {
    /// Anchor name in parent branch where the fork happened.
    pub at_anchor: String,
    /// Child session branch created by the fork.
    pub branch:    SessionBranch,
}

/// A session branch with its anchors and child forks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionBranch {
    /// Session key for this branch.
    pub session_key: String,
    /// Optional session title.
    pub title:       Option<String>,
    /// Anchors in append order.
    pub anchors:     Vec<AnchorNode>,
    /// Child branches forked from anchors in this session.
    pub forks:       Vec<ForkEdge>,
}

/// Full anchor tree rooted at the original session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnchorTree {
    /// Root branch (un-forked ancestor).
    pub root:            SessionBranch,
    /// Current session key for "you are here" highlighting.
    pub current_session: String,
}
