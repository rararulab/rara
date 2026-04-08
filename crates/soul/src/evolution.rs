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

//! Soul evolution — snapshot management and boundary validation.
//!
//! The actual LLM-driven evolution logic is not yet implemented. This module
//! provides the infrastructure for snapshotting and validating soul changes.

use std::path::Path;

use snafu::ResultExt;
use tracing::{info, warn};

use crate::{
    error::{IoSnafu, Result},
    file::{SoulFile, SoulFrontmatter},
};

/// Create a snapshot of the current soul file.
///
/// Saves to `{snapshots_dir}/v{version}.md`.
pub fn create_snapshot(soul: &SoulFile, snapshots_dir: &Path) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(snapshots_dir).context(IoSnafu {
        path: snapshots_dir.to_path_buf(),
    })?;

    let filename = format!("v{}.md", soul.frontmatter.version);
    let path = snapshots_dir.join(&filename);

    let content = soul.to_string()?;
    std::fs::write(&path, content).context(IoSnafu { path: path.clone() })?;

    info!(
        version = soul.frontmatter.version,
        path = %path.display(),
        "soul snapshot created"
    );

    Ok(path)
}

/// List available snapshot versions in a snapshots directory.
///
/// Returns versions sorted in ascending order.
pub fn list_snapshots(snapshots_dir: &Path) -> Result<Vec<u32>> {
    if !snapshots_dir.exists() {
        return Ok(vec![]);
    }

    let mut versions = Vec::new();
    let entries = std::fs::read_dir(snapshots_dir).context(IoSnafu {
        path: snapshots_dir.to_path_buf(),
    })?;

    for entry in entries {
        let entry = entry.context(IoSnafu {
            path: snapshots_dir.to_path_buf(),
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(ver_str) = name.strip_prefix('v').and_then(|s| s.strip_suffix(".md")) {
            if let Ok(v) = ver_str.parse::<u32>() {
                versions.push(v);
            }
        }
    }

    versions.sort_unstable();
    Ok(versions)
}

/// Load a specific snapshot by version.
pub fn load_snapshot(snapshots_dir: &Path, version: u32) -> Result<Option<SoulFile>> {
    let path = snapshots_dir.join(format!("v{version}.md"));
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).context(IoSnafu { path: path.clone() })?;
    let soul = SoulFile::parse(&content)?;
    Ok(Some(soul))
}

/// Validate that a proposed soul change respects the boundaries defined in the
/// frontmatter.
///
/// Returns a list of violation descriptions. Empty list means valid.
pub fn validate_boundaries(original: &SoulFrontmatter, proposed: &SoulFrontmatter) -> Vec<String> {
    let mut violations = Vec::new();

    // Check immutable traits are preserved
    for required_trait in &original.boundaries.immutable_traits {
        let found = proposed
            .boundaries
            .immutable_traits
            .iter()
            .any(|t| t == required_trait)
            || proposed.personality.iter().any(|t| t == required_trait);
        if !found {
            violations.push(format!(
                "immutable trait '{}' missing from proposed soul",
                required_trait
            ));
        }
    }

    // Check formality bounds are respected
    if let (Some(orig_min), Some(prop_min)) = (
        original.boundaries.min_formality,
        proposed.boundaries.min_formality,
    ) {
        if prop_min < orig_min {
            violations.push(format!(
                "min_formality lowered from {} to {} (not allowed)",
                orig_min, prop_min
            ));
        }
    }

    if let (Some(orig_max), Some(prop_max)) = (
        original.boundaries.max_formality,
        proposed.boundaries.max_formality,
    ) {
        if prop_max > orig_max {
            violations.push(format!(
                "max_formality raised from {} to {} (not allowed)",
                orig_max, prop_max
            ));
        }
    }

    if !violations.is_empty() {
        warn!(?violations, "soul evolution boundary violations detected");
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::SoulFile;

    fn test_soul() -> SoulFile {
        SoulFile::parse(
            "---\nname: rara\nversion: 3\npersonality:\n- warm\nboundaries:\n  \
             immutable_traits:\n  - honest\n  min_formality: 2\n  max_formality: 8\n---\n## \
             Body\n\nTest.\n",
        )
        .unwrap()
    }

    #[test]
    fn snapshot_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let soul = test_soul();

        let path = create_snapshot(&soul, dir.path()).unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().contains("v3.md"));

        let versions = list_snapshots(dir.path()).unwrap();
        assert_eq!(versions, vec![3]);

        let loaded = load_snapshot(dir.path(), 3).unwrap().unwrap();
        assert_eq!(loaded.frontmatter.name, "rara");
        assert_eq!(loaded.frontmatter.version, 3);
    }

    #[test]
    fn load_nonexistent_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_snapshot(dir.path(), 99).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn list_snapshots_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let versions = list_snapshots(dir.path()).unwrap();
        assert!(versions.is_empty());
    }

    #[test]
    fn list_snapshots_nonexistent_dir() {
        let versions = list_snapshots(Path::new("/nonexistent/snapshots")).unwrap();
        assert!(versions.is_empty());
    }

    #[test]
    fn validate_boundaries_all_good() {
        let soul = test_soul();
        let proposed = test_soul();
        let violations = validate_boundaries(&soul.frontmatter, &proposed.frontmatter);
        assert!(violations.is_empty());
    }

    #[test]
    fn validate_boundaries_missing_immutable_trait() {
        let soul = test_soul();
        let mut proposed = test_soul();
        proposed.frontmatter.boundaries.immutable_traits.clear();
        proposed.frontmatter.personality.clear();

        let violations = validate_boundaries(&soul.frontmatter, &proposed.frontmatter);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].contains("honest"));
    }

    #[test]
    fn validate_boundaries_formality_violation() {
        let soul = test_soul();
        let mut proposed = test_soul();
        proposed.frontmatter.boundaries.min_formality =
            Some(crate::score::StyleScore::new(1).unwrap()); // lower than original 2
        proposed.frontmatter.boundaries.max_formality =
            Some(crate::score::StyleScore::new(10).unwrap()); // higher than original 8

        let violations = validate_boundaries(&soul.frontmatter, &proposed.frontmatter);
        assert_eq!(violations.len(), 2);
    }
}
