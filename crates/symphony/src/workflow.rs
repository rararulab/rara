use snafu::ensure;

use crate::error::{ConfigSnafu, Result};
use crate::event::TrackedIssue;

/// Parsed representation of a WORKFLOW.md file.
#[derive(Debug, Clone)]
pub struct WorkflowFile {
    /// Parsed YAML front matter as a JSON Value, or `Value::Null` if absent.
    pub front_matter: serde_json::Value,
    /// The Markdown body after the front matter (trimmed).
    pub prompt_template: String,
}

/// Context for rendering a prompt template.
pub struct PromptContext<'a> {
    pub issue: &'a TrackedIssue,
    pub attempt: Option<u32>,
}

/// Parse a WORKFLOW.md file content into front matter + prompt template.
///
/// If the content starts with `---\n`, the text between the first and second
/// `---\n` is parsed as YAML. The remainder is the prompt template.
/// If no front matter delimiter is found, the entire content is the template.
pub fn parse_workflow(content: &str) -> Result<WorkflowFile> {
    // Check for YAML front matter delimited by "---".
    if content.starts_with("---\n") || content.starts_with("---\r\n") {
        // Find the closing delimiter after the opening one.
        let after_open = if content.starts_with("---\r\n") {
            5
        } else {
            4
        };
        let rest = &content[after_open..];

        // Look for the closing "---" at the start of a line.
        if let Some(close_pos) = find_closing_delimiter(rest) {
            let yaml_str = &rest[..close_pos];
            let after_close = &rest[close_pos..];
            // Skip the closing "---" line.
            let body = after_close
                .strip_prefix("---\r\n")
                .or_else(|| after_close.strip_prefix("---\n"))
                .or_else(|| after_close.strip_prefix("---"))
                .unwrap_or(after_close);

            // Parse YAML.
            let yaml_value: serde_yaml::Value =
                serde_yaml::from_str(yaml_str).map_err(|e| ConfigSnafu {
                    message: format!("invalid YAML front matter: {e}"),
                }.build())?;

            // Must be a mapping/object (or null for empty front matter).
            let front_matter = match &yaml_value {
                serde_yaml::Value::Mapping(_) | serde_yaml::Value::Null => {
                    serde_json::to_value(yaml_value).map_err(|e| ConfigSnafu {
                        message: format!("failed to convert YAML to JSON: {e}"),
                    }.build())?
                }
                _ => {
                    ensure!(false, ConfigSnafu {
                        message: "YAML front matter must be a mapping/object".to_owned(),
                    });
                    unreachable!()
                }
            };

            return Ok(WorkflowFile {
                front_matter,
                prompt_template: body.trim().to_owned(),
            });
        }
    }

    // No front matter — entire content is the template.
    Ok(WorkflowFile {
        front_matter: serde_json::Value::Null,
        prompt_template: content.trim().to_owned(),
    })
}

/// Find the position of the closing `---` delimiter in the remaining text.
/// Returns the byte offset of the `---` line start.
fn find_closing_delimiter(text: &str) -> Option<usize> {
    let mut offset = 0;
    for line in text.lines() {
        if line == "---" {
            return Some(offset);
        }
        // Account for the line + newline character(s).
        offset += line.len();
        // Skip past the newline.
        if offset < text.len() {
            if text.as_bytes().get(offset) == Some(&b'\r') {
                offset += 1;
            }
            if text.as_bytes().get(offset) == Some(&b'\n') {
                offset += 1;
            }
        }
    }
    None
}

/// Render a prompt template with the given context.
///
/// Replaces `{{issue.number}}`, `{{issue.title}}`, `{{issue.body}}`,
/// `{{issue.labels}}`, `{{issue.repo}}`, `{{issue.id}}`, `{{attempt}}`
/// variables.
///
/// For `{% if attempt %}...{% endif %}` blocks, includes content only if
/// `attempt` is `Some`.
pub fn render_prompt(template: &str, ctx: &PromptContext<'_>) -> Result<String> {
    let mut result = template.to_owned();

    // Handle {% if attempt %}...{% endif %} conditional blocks first.
    result = render_conditionals(&result, ctx);

    // Replace variables.
    result = result.replace("{{issue.number}}", &ctx.issue.number.to_string());
    result = result.replace("{{issue.title}}", &ctx.issue.title);
    result = result.replace(
        "{{issue.body}}",
        ctx.issue.body.as_deref().unwrap_or(""),
    );
    result = result.replace("{{issue.labels}}", &ctx.issue.labels.join(", "));
    result = result.replace("{{issue.repo}}", &ctx.issue.repo);
    result = result.replace("{{issue.id}}", &ctx.issue.id);
    result = result.replace(
        "{{attempt}}",
        &ctx.attempt
            .map(|n| n.to_string())
            .unwrap_or_default(),
    );

    Ok(result)
}

/// Process `{% if attempt %}...{% endif %}` conditional blocks.
fn render_conditionals(template: &str, ctx: &PromptContext<'_>) -> String {
    let mut result = String::with_capacity(template.len());
    let mut remaining = template;

    while let Some(if_start) = remaining.find("{% if attempt %}") {
        // Add everything before the {% if attempt %} tag.
        result.push_str(&remaining[..if_start]);

        let after_if = &remaining[if_start + "{% if attempt %}".len()..];

        if let Some(endif_pos) = after_if.find("{% endif %}") {
            let block_content = &after_if[..endif_pos];
            let after_endif = &after_if[endif_pos + "{% endif %}".len()..];

            if ctx.attempt.is_some() {
                result.push_str(block_content);
            }

            remaining = after_endif;
        } else {
            // No matching endif — leave as-is.
            result.push_str(&remaining[if_start..]);
            remaining = "";
            break;
        }
    }

    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::IssueState;
    use chrono::Utc;

    fn sample_issue() -> TrackedIssue {
        TrackedIssue {
            id: "owner/repo#42".to_owned(),
            repo: "owner/repo".to_owned(),
            number: 42,
            title: "Add widget support".to_owned(),
            body: Some("We need widgets for the dashboard.".to_owned()),
            labels: vec!["enhancement".to_owned(), "core".to_owned()],
            priority: 1,
            state: IssueState::Active,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn parse_workflow_with_front_matter() {
        let content = r#"---
tracker:
  kind: github
hooks:
  after_create: "cargo fetch"
---

You are working on issue #{{issue.number}}: {{issue.title}}
"#;
        let wf = parse_workflow(content).unwrap();

        assert!(wf.front_matter.is_object());
        assert_eq!(wf.front_matter["tracker"]["kind"], "github");
        assert_eq!(wf.front_matter["hooks"]["after_create"], "cargo fetch");
        assert!(wf.prompt_template.contains("{{issue.number}}"));
        assert!(wf.prompt_template.starts_with("You are working on"));
    }

    #[test]
    fn parse_workflow_without_front_matter() {
        let content = "Just a plain template\nwith multiple lines.";
        let wf = parse_workflow(content).unwrap();

        assert!(wf.front_matter.is_null());
        assert_eq!(wf.prompt_template, "Just a plain template\nwith multiple lines.");
    }

    #[test]
    fn parse_workflow_empty_body() {
        let content = "---\nkey: value\n---\n";
        let wf = parse_workflow(content).unwrap();

        assert!(wf.front_matter.is_object());
        assert_eq!(wf.front_matter["key"], "value");
        assert!(wf.prompt_template.is_empty());
    }

    #[test]
    fn render_simple_template() {
        let template = "Issue #{{issue.number}}: {{issue.title}}\n\n{{issue.body}}\n\nLabels: {{issue.labels}}\nRepo: {{issue.repo}}\nID: {{issue.id}}";
        let issue = sample_issue();
        let ctx = PromptContext {
            issue: &issue,
            attempt: None,
        };

        let rendered = render_prompt(template, &ctx).unwrap();

        assert!(rendered.contains("Issue #42: Add widget support"));
        assert!(rendered.contains("We need widgets for the dashboard."));
        assert!(rendered.contains("Labels: enhancement, core"));
        assert!(rendered.contains("Repo: owner/repo"));
        assert!(rendered.contains("ID: owner/repo#42"));
    }

    #[test]
    fn render_conditional_attempt_present() {
        let template = "Start\n{% if attempt %}Retry attempt {{attempt}}.{% endif %}\nEnd";
        let issue = sample_issue();
        let ctx = PromptContext {
            issue: &issue,
            attempt: Some(3),
        };

        let rendered = render_prompt(template, &ctx).unwrap();

        assert!(rendered.contains("Retry attempt 3."));
        assert!(rendered.contains("Start"));
        assert!(rendered.contains("End"));
    }

    #[test]
    fn render_conditional_attempt_absent() {
        let template = "Start\n{% if attempt %}Retry attempt {{attempt}}.{% endif %}\nEnd";
        let issue = sample_issue();
        let ctx = PromptContext {
            issue: &issue,
            attempt: None,
        };

        let rendered = render_prompt(template, &ctx).unwrap();

        assert!(!rendered.contains("Retry attempt"));
        assert!(rendered.contains("Start"));
        assert!(rendered.contains("End"));
    }
}
