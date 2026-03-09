use std::path::PathBuf;

use chrono::Utc;
use rara_symphony::{
    agent::{AgentTask, RalphAgent},
    config::{AgentConfig, RepoConfig},
    tracker::{IssueState, TrackedIssue},
};

#[test]
fn repo_config_defaults_workspace_root_under_rara_config_dir() {
    let repo = RepoConfig::builder()
        .name("rararulab/rara".to_owned())
        .url("https://github.com/rararulab/rara".to_owned())
        .repo_path(PathBuf::from("/tmp/rara"))
        .active_labels(vec!["symphony:ready".to_owned()])
        .build();

    let repo_path = repo
        .repo_path
        .as_ref()
        .expect("repo_path should be preserved");
    let workspace_root = repo
        .effective_workspace_root()
        .expect("workspace_root should default from repo_path");

    assert_eq!(repo_path, &PathBuf::from("/tmp/rara"));
    assert_eq!(
        workspace_root,
        rara_paths::config_dir()
            .join("ralpha/worktress")
            .join("rararulab/rara")
            .join("worktrees")
    );
}

#[test]
fn agent_config_builds_ralph_run_command() {
    let agent = AgentConfig::builder()
        .command("ralph".to_owned())
        .config_file(PathBuf::from("/tmp/ralph.yml"))
        .extra_args(vec!["--autonomous".to_owned()])
        .build();

    let args = agent.command_args();

    assert_eq!(
        args,
        vec![
            "run".to_owned(),
            "-c".to_owned(),
            "/tmp/ralph.yml".to_owned(),
            "--no-tui".to_owned(),
            "--autonomous".to_owned(),
        ]
    );
}

#[test]
fn default_prompt_requires_push_pr_and_linear_comment() {
    let agent = RalphAgent::new(AgentConfig::default());
    let task = AgentTask {
        issue:            TrackedIssue {
            id:         "lin_123".to_owned(),
            identifier: "RAR-123".to_owned(),
            repo:       "rararulab/rara".to_owned(),
            number:     123,
            title:      "Add merge tracking".to_owned(),
            body:       Some("Track PR merge status before closing the Linear issue.".to_owned()),
            labels:     vec![],
            priority:   1,
            state:      IssueState::Active,
            created_at: Utc::now(),
        },
        attempt:          None,
        workflow_content: None,
    };

    let prompt = agent.build_prompt(&task);

    assert!(prompt.contains("push"));
    assert!(prompt.contains("pull request"));
    assert!(prompt.contains("Linear"));
    assert!(prompt.contains("comment"));
    assert!(prompt.contains("RAR-123"));
}
