use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// YAML frontmatter parsed from a skill `.md` file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tools: Vec<String>,
    pub trigger: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// A fully parsed skill, combining metadata with prompt body and source path.
#[derive(Debug, Clone)]
pub struct Skill {
    pub metadata: SkillMetadata,
    pub prompt: String,
    pub source_path: PathBuf,
}

impl Skill {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.metadata.description
    }

    #[must_use]
    pub fn tools(&self) -> &[String] {
        &self.metadata.tools
    }

    #[must_use]
    pub fn trigger_pattern(&self) -> Option<&str> {
        self.metadata.trigger.as_deref()
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.metadata.enabled
    }
}
