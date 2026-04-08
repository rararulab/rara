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

use std::collections::HashMap;

use chromiumoxide::cdp::browser_protocol::dom::BackendNodeId;

use super::error::{BrowserResult, RefNotFoundSnafu};

/// Maps snapshot ref IDs (e.g. `"1"`, `"42"`) to CDP `BackendNodeId` values.
///
/// Rebuilt on every snapshot — old refs are invalidated when a new snapshot is
/// taken. Tools like `browser-click` and `browser-type` resolve their `ref`
/// parameter through this map.
#[derive(Debug, Clone)]
pub struct RefMap {
    /// ref string → CDP BackendNodeId
    refs:    HashMap<String, BackendNodeId>,
    /// Next ref counter (reset on each rebuild).
    counter: u32,
}

impl RefMap {
    /// Create an empty ref map.
    #[must_use]
    pub fn new() -> Self {
        Self {
            refs:    HashMap::new(),
            counter: 0,
        }
    }

    /// Clear all mappings and reset the counter. Called before rebuilding from
    /// a new accessibility tree snapshot.
    pub fn clear(&mut self) {
        self.refs.clear();
        self.counter = 0;
    }

    /// Insert a new node and return its assigned ref ID.
    pub fn insert(&mut self, backend_node_id: BackendNodeId) -> String {
        self.counter += 1;
        let ref_id = self.counter.to_string();
        self.refs.insert(ref_id.clone(), backend_node_id);
        ref_id
    }

    /// Look up a CDP `BackendNodeId` by ref string.
    pub fn resolve(&self, ref_id: &str) -> BrowserResult<BackendNodeId> {
        self.refs
            .get(ref_id)
            .copied()
            .ok_or_else(|| RefNotFoundSnafu { ref_id }.build())
    }

    /// Number of refs in the current snapshot.
    #[must_use]
    pub fn len(&self) -> usize { self.refs.len() }

    /// Whether the map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.refs.is_empty() }
}

impl Default for RefMap {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_resolve() {
        let mut map = RefMap::new();
        let node_id = BackendNodeId::new(42);
        let ref_id = map.insert(node_id);
        assert_eq!(ref_id, "1");
        assert_eq!(map.resolve("1").unwrap(), node_id);
    }

    #[test]
    fn sequential_ids() {
        let mut map = RefMap::new();
        let r1 = map.insert(BackendNodeId::new(10));
        let r2 = map.insert(BackendNodeId::new(20));
        let r3 = map.insert(BackendNodeId::new(30));
        assert_eq!(r1, "1");
        assert_eq!(r2, "2");
        assert_eq!(r3, "3");
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn clear_resets() {
        let mut map = RefMap::new();
        map.insert(BackendNodeId::new(1));
        map.insert(BackendNodeId::new(2));
        map.clear();
        assert!(map.is_empty());
        let ref_id = map.insert(BackendNodeId::new(3));
        assert_eq!(ref_id, "1", "counter should reset after clear");
    }

    #[test]
    fn resolve_missing_ref() {
        let map = RefMap::new();
        let err = map.resolve("999").unwrap_err();
        assert!(err.to_string().contains("999"));
    }
}
