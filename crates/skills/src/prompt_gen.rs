use crate::types::SkillMetadata;

/// Generate the `<available_skills>` XML block for injection into the system prompt.
pub fn generate_skills_prompt(skills: &[SkillMetadata]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    use crate::types::SkillSource;

    let mut out = String::from("## Available Skills\n\n<available_skills>\n");
    for skill in skills {
        let is_plugin = skill.source.as_ref() == Some(&SkillSource::Plugin);
        let path_display = if is_plugin {
            skill.path.display().to_string()
        } else {
            skill.path.join("SKILL.md").display().to_string()
        };
        out.push_str(&format!(
            "<skill name=\"{}\" source=\"{}\" path=\"{}\">\n{}\n</skill>\n",
            skill.name,
            if is_plugin {
                "plugin"
            } else {
                "skill"
            },
            path_display,
            skill.description,
        ));
    }
    out.push_str("</available_skills>\n\n");
    out.push_str(
        "To activate a skill, read its SKILL.md file (or the plugin's .md file at the given path) for full instructions.\n\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn test_empty_skills_produces_empty_string() {
        assert_eq!(generate_skills_prompt(&[]), "");
    }

    #[test]
    fn test_single_skill_prompt() {
        let skills = vec![SkillMetadata {
            name: "commit".into(),
            description: "Create git commits".into(),
            license: None,
            compatibility: None,
            allowed_tools: vec![],
            homepage: None,
            dockerfile: None,
            requires: Default::default(),
            path: PathBuf::from("/home/user/.moltis/skills/commit"),
            source: None,
        }];
        let prompt = generate_skills_prompt(&skills);
        assert!(prompt.contains("<available_skills>"));
        assert!(prompt.contains("name=\"commit\""));
        assert!(prompt.contains("Create git commits"));
        assert!(prompt.contains("SKILL.md"));
        assert!(prompt.contains("</available_skills>"));
    }

    #[test]
    fn test_multiple_skills() {
        let skills = vec![
            SkillMetadata {
                name: "commit".into(),
                description: "Commits".into(),
                license: None,
                compatibility: None,
                allowed_tools: vec![],
                homepage: None,
                dockerfile: None,
                requires: Default::default(),
                path: PathBuf::from("/a"),
                source: None,
            },
            SkillMetadata {
                name: "review".into(),
                description: "Reviews".into(),
                license: None,
                compatibility: None,
                allowed_tools: vec![],
                homepage: None,
                dockerfile: None,
                requires: Default::default(),
                path: PathBuf::from("/b"),
                source: None,
            },
        ];
        let prompt = generate_skills_prompt(&skills);
        assert!(prompt.contains("name=\"commit\""));
        assert!(prompt.contains("name=\"review\""));
    }
}
