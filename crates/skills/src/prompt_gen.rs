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

//! System prompt generation for LLM skill injection.
//!
//! Generates an `<available_skills>` XML block listing all discovered skills
//! with their names, sources, paths, and descriptions, suitable for injection
//! into the LLM system prompt.

use crate::types::SkillMetadata;

/// Generate the `<available_skills>` XML block for injection into the system
/// prompt.
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
            if is_plugin { "plugin" } else { "skill" },
            path_display,
            skill.description,
        ));
    }
    out.push_str("</available_skills>\n\n");
    out.push_str(
        "To activate a skill, read its SKILL.md file (or the plugin's .md file at the given path) \
         for full instructions.\n\n",
    );
    out.push_str(
        "IMPORTANT: Some skills were written for other environments and may reference tools you \
         don't have (e.g. WebSearch, WebFetch, Skill, Task). Ignore those tool names. Use YOUR \
         actual tools instead — for example, use `http_fetch` for any web/HTTP access, `bash` for \
         shell commands, `read_file`/`write_file` for file operations. You DO have internet \
         access via `http_fetch`. Never claim you lack capabilities just because a skill \
         references an unfamiliar tool name.\n\n",
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
            name:          "commit".into(),
            description:   "Create git commits".into(),
            license:       None,
            compatibility: None,
            allowed_tools: vec![],
            homepage:      None,
            dockerfile:    None,
            requires:      Default::default(),
            path:          PathBuf::from("/home/user/.moltis/skills/commit"),
            source:        None,
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
                name:          "commit".into(),
                description:   "Commits".into(),
                license:       None,
                compatibility: None,
                allowed_tools: vec![],
                homepage:      None,
                dockerfile:    None,
                requires:      Default::default(),
                path:          PathBuf::from("/a"),
                source:        None,
            },
            SkillMetadata {
                name:          "review".into(),
                description:   "Reviews".into(),
                license:       None,
                compatibility: None,
                allowed_tools: vec![],
                homepage:      None,
                dockerfile:    None,
                requires:      Default::default(),
                path:          PathBuf::from("/b"),
                source:        None,
            },
        ];
        let prompt = generate_skills_prompt(&skills);
        assert!(prompt.contains("name=\"commit\""));
        assert!(prompt.contains("name=\"review\""));
    }
}
