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

//! Soul file parsing — YAML frontmatter + markdown body.

use serde::{Deserialize, Serialize};
use snafu::ResultExt;

use crate::error::{ParseFrontmatterSnafu, Result};

/// Parsed soul file: frontmatter metadata + markdown body.
#[derive(Debug, Clone)]
pub struct SoulFile {
    pub frontmatter: SoulFrontmatter,
    pub body:        String,
}

/// YAML frontmatter of a soul file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoulFrontmatter {
    pub name:        String,
    #[serde(default = "default_version")]
    pub version:     u32,
    #[serde(default)]
    pub personality:  Vec<String>,
    #[serde(default)]
    pub boundaries:  Boundaries,
    #[serde(default)]
    pub evolution:   EvolutionConfig,
}

fn default_version() -> u32 { 1 }

/// Hard constraints that survive any evolution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Boundaries {
    #[serde(default)]
    pub immutable_traits: Vec<String>,
    pub min_formality:    Option<u8>,
    pub max_formality:    Option<u8>,
}

/// Evolution feature flags.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvolutionConfig {
    #[serde(default)]
    pub enabled:              bool,
    #[serde(default)]
    pub speaking_style_drift: bool,
    #[serde(default)]
    pub trait_emergence:      bool,
    #[serde(default)]
    pub mood_tracking:        bool,
}

impl SoulFile {
    /// Parse a soul file from its raw text content.
    ///
    /// Expected format:
    /// ```text
    /// ---
    /// name: rara
    /// ...frontmatter fields...
    /// ---
    ///
    /// ## Background
    /// ...markdown body...
    /// ```
    pub fn parse(content: &str) -> Result<Self> {
        let (frontmatter_str, body) = split_frontmatter(content);

        let frontmatter: SoulFrontmatter =
            serde_yaml::from_str(frontmatter_str).context(ParseFrontmatterSnafu)?;

        Ok(Self {
            frontmatter,
            body: body.to_string(),
        })
    }

    /// Serialize back to soul file format (frontmatter + body).
    pub fn to_string(&self) -> Result<String> {
        let yaml = serde_yaml::to_string(&self.frontmatter)
            .context(ParseFrontmatterSnafu)?;
        Ok(format!("---\n{yaml}---\n{}", self.body))
    }
}

/// Split raw content into (frontmatter_yaml, body_markdown).
///
/// If no valid `---` delimiters are found, the entire content is treated as
/// body with empty frontmatter.
fn split_frontmatter(content: &str) -> (&str, &str) {
    let trimmed = content.trim_start();

    if !trimmed.starts_with("---") {
        return ("", content);
    }

    // Skip the opening "---" line
    let after_opening = match trimmed.strip_prefix("---") {
        Some(rest) => rest.trim_start_matches([' ', '\t']),
        None => return ("", content),
    };

    // Skip the newline after "---"
    let after_newline = if after_opening.starts_with('\n') {
        &after_opening[1..]
    } else if after_opening.starts_with("\r\n") {
        &after_opening[2..]
    } else {
        return ("", content);
    };

    // Find the closing "---"
    if let Some(end_pos) = find_closing_delimiter(after_newline) {
        let frontmatter = &after_newline[..end_pos];
        let rest = &after_newline[end_pos..];
        // Skip the closing "---" line
        let body = rest
            .strip_prefix("---")
            .unwrap_or(rest)
            .trim_start_matches([' ', '\t']);
        let body = if body.starts_with('\n') {
            &body[1..]
        } else if body.starts_with("\r\n") {
            &body[2..]
        } else {
            body
        };
        (frontmatter, body)
    } else {
        ("", content)
    }
}

/// Find the position of a line that is exactly `---` (the closing delimiter).
fn find_closing_delimiter(s: &str) -> Option<usize> {
    let mut pos = 0;
    for line in s.lines() {
        if line.trim() == "---" {
            return Some(pos);
        }
        // +1 for the newline character
        pos += line.len() + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_soul_file() {
        let content = r#"---
name: rara
version: 2
personality:
  - 温暖
  - 好奇
boundaries:
  immutable_traits:
    - 诚实
  min_formality: 2
  max_formality: 7
evolution:
  enabled: true
  mood_tracking: true
---

## Background

Rara is a warm, curious assistant.

## Speaking Style

Natural and friendly.
"#;
        let soul = SoulFile::parse(content).unwrap();
        assert_eq!(soul.frontmatter.name, "rara");
        assert_eq!(soul.frontmatter.version, 2);
        assert_eq!(soul.frontmatter.personality, vec!["温暖", "好奇"]);
        assert_eq!(soul.frontmatter.boundaries.immutable_traits, vec!["诚实"]);
        assert_eq!(soul.frontmatter.boundaries.min_formality, Some(2));
        assert_eq!(soul.frontmatter.boundaries.max_formality, Some(7));
        assert!(soul.frontmatter.evolution.enabled);
        assert!(soul.frontmatter.evolution.mood_tracking);
        assert!(soul.body.contains("## Background"));
        assert!(soul.body.contains("## Speaking Style"));
    }

    #[test]
    fn parse_minimal_frontmatter() {
        let content = "---\nname: test\n---\nHello world";
        let soul = SoulFile::parse(content).unwrap();
        assert_eq!(soul.frontmatter.name, "test");
        assert_eq!(soul.frontmatter.version, 1);
        assert_eq!(soul.body, "Hello world");
    }

    #[test]
    fn parse_no_frontmatter_returns_error() {
        let content = "Just some markdown without frontmatter";
        // Empty frontmatter string -> YAML parse error (name is required)
        assert!(SoulFile::parse(content).is_err());
    }

    #[test]
    fn roundtrip_serialization() {
        let content = "---\nname: rara\nversion: 1\npersonality:\n- warm\nboundaries:\n  immutable_traits:\n  - honest\nevolution:\n  enabled: true\n---\n## Body\n\nSome text.\n";
        let soul = SoulFile::parse(content).unwrap();
        let serialized = soul.to_string().unwrap();
        let reparsed = SoulFile::parse(&serialized).unwrap();
        assert_eq!(reparsed.frontmatter.name, "rara");
        assert!(reparsed.body.contains("## Body"));
    }
}
