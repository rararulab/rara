use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use tracing::{info, warn};

use crate::error::SkillError;
use crate::loader::discover_skills;
use crate::types::Skill;

/// In-memory registry of loaded skills with trigger pattern matching.
pub struct SkillRegistry {
    skills: HashMap<String, Skill>,
    triggers: HashMap<String, Regex>,
}

impl SkillRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
            triggers: HashMap::new(),
        }
    }

    /// Load skills from multiple directories.
    ///
    /// Later directories have higher precedence: if two directories contain a
    /// skill with the same name, the one from the later directory wins.
    /// Directories that do not exist are silently skipped.
    pub fn load_from_dirs(dirs: &[&Path]) -> Result<Self, SkillError> {
        let mut registry = Self::new();
        for dir in dirs {
            if !dir.exists() {
                continue;
            }
            for result in discover_skills(dir) {
                match result {
                    Ok(skill) => {
                        if skill.is_enabled() {
                            registry.insert(skill);
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to load skill, skipping");
                    }
                }
            }
        }
        info!(count = registry.skills.len(), "skills loaded");
        Ok(registry)
    }

    /// Insert (or replace) a skill in the registry.
    pub fn insert(&mut self, skill: Skill) {
        if let Some(pattern) = skill.trigger_pattern() {
            match Regex::new(pattern) {
                Ok(re) => {
                    self.triggers.insert(skill.name().to_owned(), re);
                }
                Err(e) => {
                    warn!(skill = skill.name(), error = %e, "invalid trigger regex");
                }
            }
        }
        self.skills.insert(skill.name().to_owned(), skill);
    }

    /// Remove a skill by name, returning it if it existed.
    pub fn remove(&mut self, name: &str) -> Option<Skill> {
        self.triggers.remove(name);
        self.skills.remove(name)
    }

    /// Look up a skill by exact name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// Return all skills (enabled and disabled).
    #[must_use]
    pub fn list_all(&self) -> Vec<&Skill> {
        self.skills.values().collect()
    }

    /// Return only enabled skills.
    #[must_use]
    pub fn list_enabled(&self) -> Vec<&Skill> {
        self.skills.values().filter(|s| s.is_enabled()).collect()
    }

    /// Find skills whose trigger pattern matches the given text.
    #[must_use]
    pub fn match_triggers(&self, text: &str) -> Vec<&Skill> {
        self.triggers
            .iter()
            .filter(|(_, re)| re.is_match(text))
            .filter_map(|(name, _)| self.skills.get(name))
            .collect()
    }

    /// Generate compact XML listing enabled skills for system prompt injection.
    #[must_use]
    pub fn to_prompt_xml(&self) -> String {
        let enabled: Vec<_> = self.list_enabled();
        if enabled.is_empty() {
            return String::new();
        }

        let mut xml = String::from("<available_skills>\n");
        for skill in &enabled {
            xml.push_str(&format!(
                "<skill name=\"{}\" description=\"{}\" />\n",
                escape_xml(skill.name()),
                escape_xml(skill.description()),
            ));
        }
        xml.push_str("</available_skills>");
        xml
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;
    use crate::types::{Skill, SkillMetadata};

    fn make_skill(name: &str, description: &str, trigger: Option<&str>, enabled: bool) -> Skill {
        Skill {
            metadata: SkillMetadata {
                name: name.to_owned(),
                description: description.to_owned(),
                tools: vec![],
                trigger: trigger.map(ToOwned::to_owned),
                enabled,
            },
            prompt: format!("Prompt for {name}"),
            source_path: PathBuf::from(format!("/fake/{name}.md")),
        }
    }

    fn write_skill_file(dir: &Path, filename: &str, content: &str) {
        fs::write(dir.join(filename), content).unwrap();
    }

    #[test]
    fn load_from_empty_directory() {
        let dir = TempDir::new().unwrap();
        let registry = SkillRegistry::load_from_dirs(&[dir.path()]).unwrap();
        assert!(registry.list_all().is_empty());
    }

    #[test]
    fn load_from_nonexistent_directory() {
        let registry =
            SkillRegistry::load_from_dirs(&[Path::new("/nonexistent/path/skills")]).unwrap();
        assert!(registry.list_all().is_empty());
    }

    #[test]
    fn insert_and_retrieve_skill() {
        let mut registry = SkillRegistry::new();
        let skill = make_skill("test", "Test skill", None, true);
        registry.insert(skill);

        let retrieved = registry.get("test");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name(), "test");
    }

    #[test]
    fn remove_skill() {
        let mut registry = SkillRegistry::new();
        registry.insert(make_skill("to-remove", "Remove me", Some("remove"), true));

        assert!(registry.get("to-remove").is_some());

        let removed = registry.remove("to-remove");
        assert!(removed.is_some());
        assert!(registry.get("to-remove").is_none());
        assert!(registry.match_triggers("remove").is_empty());
    }

    #[test]
    fn trigger_matching() {
        let mut registry = SkillRegistry::new();
        registry.insert(make_skill(
            "job-search",
            "Job search",
            Some("(?i)找工作|job search"),
            true,
        ));

        // Exact match.
        assert_eq!(registry.match_triggers("找工作").len(), 1);

        // Case-insensitive match via (?i) flag.
        assert_eq!(registry.match_triggers("Job Search").len(), 1);
        assert_eq!(registry.match_triggers("JOB SEARCH").len(), 1);

        // Substring match (regex is not anchored).
        assert_eq!(registry.match_triggers("I want to job search now").len(), 1);

        // No match.
        assert!(registry.match_triggers("unrelated text").is_empty());
    }

    #[test]
    fn precedence_later_dir_overrides_earlier() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();

        write_skill_file(
            dir1.path(),
            "greet.md",
            "---\nname: greet\ndescription: Version 1\n---\nPrompt v1\n",
        );
        write_skill_file(
            dir2.path(),
            "greet.md",
            "---\nname: greet\ndescription: Version 2\n---\nPrompt v2\n",
        );

        let registry =
            SkillRegistry::load_from_dirs(&[dir1.path(), dir2.path()]).unwrap();
        let skill = registry.get("greet").unwrap();
        assert_eq!(skill.description(), "Version 2");
    }

    #[test]
    fn to_prompt_xml_generates_valid_output() {
        let mut registry = SkillRegistry::new();
        registry.insert(make_skill("alpha", "Alpha skill", None, true));
        registry.insert(make_skill("beta", "Beta & <special>", None, true));

        let xml = registry.to_prompt_xml();
        assert!(xml.starts_with("<available_skills>"));
        assert!(xml.ends_with("</available_skills>"));
        assert!(xml.contains("name=\"alpha\""));
        assert!(xml.contains("Beta &amp; &lt;special&gt;"));
    }

    #[test]
    fn to_prompt_xml_empty_when_no_skills() {
        let registry = SkillRegistry::new();
        assert!(registry.to_prompt_xml().is_empty());
    }

    #[test]
    fn disabled_skills_excluded_from_list_enabled() {
        let mut registry = SkillRegistry::new();
        registry.insert(make_skill("on", "Enabled", None, true));
        registry.insert(make_skill("off", "Disabled", None, false));

        assert_eq!(registry.list_all().len(), 2);
        assert_eq!(registry.list_enabled().len(), 1);
        assert_eq!(registry.list_enabled()[0].name(), "on");
    }
}
