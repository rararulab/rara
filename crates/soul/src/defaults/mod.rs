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

//! Default soul definitions embedded in the binary.
//!
//! These serve as code fallbacks when no user-defined soul file exists on disk.
//! They also function as templates for users creating their own soul files.

/// Default soul definition for the rara agent.
pub const RARA_SOUL: &str = include_str!("rara.md");

/// Default soul definition for the nana agent.
pub const NANA_SOUL: &str = include_str!("nana.md");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::SoulFile;

    #[test]
    fn rara_default_parses() {
        let soul = SoulFile::parse(RARA_SOUL).unwrap();
        assert_eq!(soul.frontmatter.name, "rara");
        assert_eq!(soul.frontmatter.version, 1);
        assert!(!soul.frontmatter.personality.is_empty());
        assert!(!soul.frontmatter.boundaries.immutable_traits.is_empty());
        assert!(soul.frontmatter.evolution.enabled);
        assert!(soul.body.contains("## Background"));
    }

    #[test]
    fn nana_default_parses() {
        let soul = SoulFile::parse(NANA_SOUL).unwrap();
        assert_eq!(soul.frontmatter.name, "nana");
        assert_eq!(soul.frontmatter.version, 1);
        assert!(!soul.frontmatter.personality.is_empty());
        assert!(!soul.frontmatter.evolution.enabled);
        assert!(soul.body.contains("## Background"));
    }
}
