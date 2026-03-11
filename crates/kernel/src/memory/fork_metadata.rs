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

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Fork provenance extracted from session metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkMetadata {
    pub forked_from:      String,
    pub forked_at_anchor: String,
}

/// Read fork provenance from session metadata.
pub fn get_fork_metadata(metadata: &Option<Value>) -> Option<ForkMetadata> {
    let obj = metadata.as_ref()?.as_object()?;
    let forked_from = obj.get("forked_from")?.as_str()?.to_owned();
    let forked_at_anchor = obj.get("forked_at_anchor")?.as_str()?.to_owned();
    Some(ForkMetadata {
        forked_from,
        forked_at_anchor,
    })
}

/// Write fork provenance into session metadata while preserving other fields.
pub fn set_fork_metadata(metadata: &mut Option<Value>, forked_from: &str, forked_at_anchor: &str) {
    let obj = metadata
        .get_or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .expect("session metadata must be a JSON object");
    obj.insert("forked_from".into(), Value::String(forked_from.to_owned()));
    obj.insert(
        "forked_at_anchor".into(),
        Value::String(forked_at_anchor.to_owned()),
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn write_then_read_fork_metadata() {
        let mut metadata = None;
        set_fork_metadata(&mut metadata, "parent-key", "topic/design");
        let fm = get_fork_metadata(&metadata).unwrap();
        assert_eq!(fm.forked_from, "parent-key");
        assert_eq!(fm.forked_at_anchor, "topic/design");
    }

    #[test]
    fn read_missing_metadata_returns_none() {
        let metadata: Option<serde_json::Value> = None;
        assert!(get_fork_metadata(&metadata).is_none());
    }

    #[test]
    fn read_metadata_without_fork_fields_returns_none() {
        let metadata = Some(json!({"other": "data"}));
        assert!(get_fork_metadata(&metadata).is_none());
    }

    #[test]
    fn write_preserves_existing_metadata() {
        let mut metadata = Some(json!({"custom": 42}));
        set_fork_metadata(&mut metadata, "parent-key", "anchor-1");
        let v = metadata.unwrap();
        assert_eq!(v["custom"], 42);
        assert_eq!(v["forked_from"], "parent-key");
        assert_eq!(v["forked_at_anchor"], "anchor-1");
    }
}
