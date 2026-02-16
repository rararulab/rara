use std::path::Path;

use snafu::ResultExt;

use crate::error::{FrontmatterSnafu, IoSnafu, MissingFrontmatterSnafu, SkillError};
use crate::types::{Skill, SkillMetadataLegacy};

/// Split raw file content into YAML frontmatter and markdown body.
///
/// Expects the content to start with `---\n`, followed by YAML, then a closing
/// `---\n`, and finally the prompt body.
fn split_frontmatter(content: &str, path: &str) -> Result<(SkillMetadataLegacy, String), SkillError> {
    // Normalise line endings so we can search for `\n---\n` uniformly.
    let normalised = content.replace("\r\n", "\n");

    if !normalised.starts_with("---\n") {
        return MissingFrontmatterSnafu {
            path: path.to_owned(),
        }
        .fail();
    }

    // Find the closing delimiter after the opening `---\n` (skip first 4 bytes).
    let rest = &normalised[4..];
    let closing = rest.find("\n---\n").ok_or_else(|| {
        // Also accept `\n---` at end-of-file (no trailing newline after closing).
        if rest.ends_with("\n---") {
            return MissingFrontmatterSnafu {
                path: "".to_owned(),
            }
            .build();
        }
        MissingFrontmatterSnafu {
            path: path.to_owned(),
        }
        .build()
    })?;

    let yaml_str = &rest[..closing];
    let body = rest[closing + 4..].trim().to_owned(); // skip `\n---\n`

    let metadata: SkillMetadataLegacy =
        serde_yaml::from_str(yaml_str).context(FrontmatterSnafu { path })?;

    Ok((metadata, body))
}

/// Parse a single `.md` skill file into a [`Skill`].
pub fn parse_skill_file(path: &Path) -> Result<Skill, SkillError> {
    let content = std::fs::read_to_string(path).context(IoSnafu)?;
    let path_str = path.display().to_string();
    let (metadata, prompt) = split_frontmatter(&content, &path_str)?;

    Ok(Skill {
        metadata,
        prompt,
        source_path: path.to_path_buf(),
    })
}

/// Discover all `.md` skill files in `dir` (non-recursive) and attempt to parse
/// each one.
pub fn discover_skills(dir: &Path) -> Vec<Result<Skill, SkillError>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            return vec![Err(e).context(IoSnafu)];
        }
    };

    entries
        .filter_map(|entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => return Some(Err(e).context(IoSnafu)),
            };
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                Some(parse_skill_file(&path))
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn write_skill(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn parse_valid_skill_file_with_all_fields() {
        let dir = TempDir::new().unwrap();
        let content = r#"---
name: job-search
description: "Job search expert"
tools:
  - job_pipeline
  - memory_search
trigger: "找工作|job search"
enabled: true
---

You are a job search expert.
Help the user find relevant positions.
"#;
        write_skill(dir.path(), "job-search.md", content);

        let skill = parse_skill_file(&dir.path().join("job-search.md")).unwrap();
        assert_eq!(skill.name(), "job-search");
        assert_eq!(skill.description(), "Job search expert");
        assert_eq!(skill.tools(), &["job_pipeline", "memory_search"]);
        assert_eq!(skill.trigger_pattern(), Some("找工作|job search"));
        assert!(skill.is_enabled());
        assert!(skill.prompt.contains("job search expert"));
    }

    #[test]
    fn parse_skill_file_with_minimal_fields() {
        let dir = TempDir::new().unwrap();
        let content = r#"---
name: minimal
description: A minimal skill
---

Just the body.
"#;
        write_skill(dir.path(), "minimal.md", content);

        let skill = parse_skill_file(&dir.path().join("minimal.md")).unwrap();
        assert_eq!(skill.name(), "minimal");
        assert_eq!(skill.description(), "A minimal skill");
        assert!(skill.tools().is_empty());
        assert_eq!(skill.trigger_pattern(), None);
        assert!(skill.is_enabled()); // defaults to true
        assert_eq!(skill.prompt, "Just the body.");
    }

    #[test]
    fn missing_frontmatter_returns_error() {
        let dir = TempDir::new().unwrap();
        let content = "No frontmatter here, just text.";
        write_skill(dir.path(), "bad.md", content);

        let result = parse_skill_file(&dir.path().join("bad.md"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("missing frontmatter"),
            "expected missing frontmatter error, got: {err}"
        );
    }

    #[test]
    fn malformed_yaml_returns_error() {
        let dir = TempDir::new().unwrap();
        let content = "---\n[invalid yaml: :\n---\n\nBody.\n";
        write_skill(dir.path(), "malformed.md", content);

        let result = parse_skill_file(&dir.path().join("malformed.md"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("invalid frontmatter"),
            "expected frontmatter parse error, got: {err}"
        );
    }

    #[test]
    fn discover_skills_finds_md_files_only() {
        let dir = TempDir::new().unwrap();
        write_skill(
            dir.path(),
            "a.md",
            "---\nname: a\ndescription: A\n---\nBody A\n",
        );
        write_skill(
            dir.path(),
            "b.md",
            "---\nname: b\ndescription: B\n---\nBody B\n",
        );
        write_skill(dir.path(), "not-a-skill.txt", "ignored");

        let results = discover_skills(dir.path());
        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok_count, 2);
    }

    #[test]
    fn discover_skills_empty_dir() {
        let dir = TempDir::new().unwrap();
        let results = discover_skills(dir.path());
        assert!(results.is_empty());
    }
}
