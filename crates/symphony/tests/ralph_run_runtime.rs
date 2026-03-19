use std::path::PathBuf;

use chrono::Utc;
use rara_symphony::{
    agent::{AgentTask, RalphAgent, merge_core_config},
    config::{AgentConfig, RepoConfig, TrackerConfig},
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
            .join("ralpha/worktrees")
            .join("rararulab/rara")
            .join("worktrees")
    );
}

#[test]
fn agent_config_builds_ralph_run_command() {
    let agent = AgentConfig::builder()
        .command("ralph".to_owned())
        .backend("codex".to_owned())
        .core_config_file(PathBuf::from("ralph.core.yml"))
        .extra_args(vec!["--max-iterations".to_owned(), "5".to_owned()])
        .build();

    let args = agent.command_args();

    assert_eq!(
        args,
        vec![
            "run".to_owned(),
            "--autonomous".to_owned(),
            "--max-iterations".to_owned(),
            "5".to_owned(),
        ]
    );
}

#[test]
fn agent_config_builds_ralph_init_command() {
    let agent = AgentConfig::builder()
        .command("ralph".to_owned())
        .backend("claude".to_owned())
        .core_config_file(PathBuf::from("ralph.core.yml"))
        .extra_args(vec![])
        .build();

    let args = agent.init_args();

    assert_eq!(
        args,
        vec![
            "init".to_owned(),
            "--force".to_owned(),
            "--backend".to_owned(),
            "claude".to_owned(),
            "-c".to_owned(),
            "ralph.core.yml".to_owned(),
        ]
    );
}

#[test]
fn agent_config_builds_ralph_doctor_command() {
    let agent = AgentConfig::builder()
        .command("ralph".to_owned())
        .backend("claude".to_owned())
        .core_config_file(PathBuf::from("ralph.core.yml"))
        .extra_args(vec![])
        .build();

    let args = agent.doctor_args();

    assert_eq!(args, vec!["doctor".to_owned()]);
}

#[test]
fn default_agent_command_uses_autonomous_mode() {
    let agent = AgentConfig::default();
    let args = agent.command_args();

    assert_eq!(agent.backend, "codex");
    assert_eq!(agent.core_config_file, PathBuf::from("ralph.core.yml"));
    assert_eq!(args.first().map(String::as_str), Some("run"));
    assert!(args.iter().any(|arg| arg == "--autonomous"));
    assert!(!args.iter().any(|arg| arg == "--no-tui"));
    assert!(!args.iter().any(|arg| arg == "-c"));
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
    assert!(prompt.contains("linear CLI"));
    assert!(prompt.contains("implementation plan"));
    assert!(prompt.contains("reasoning"));
    assert!(prompt.contains("RAR-123"));
}

#[test]
fn tracker_config_defaults_completion_state_to_to_verify() {
    let tracker = TrackerConfig::Linear {
        api_key:               "token".to_owned(),
        team_key:              "RAR".to_owned(),
        project_slug:          None,
        endpoint:              "https://api.linear.app/graphql".to_owned(),
        active_states:         vec!["Todo".to_owned()],
        terminal_states:       vec!["Done".to_owned()],
        repo_label_prefix:     "repo:".to_owned(),
        started_issue_state:   "In Progress".to_owned(),
        completed_issue_state: "ToVerify".to_owned(),
    };

    assert_eq!(tracker.started_issue_state(), "In Progress");
    assert_eq!(tracker.completed_issue_state(), "ToVerify");
    assert_eq!(tracker.active_states(), &["Todo".to_owned()]);
}

#[test]
fn tracker_config_allows_custom_completion_state() {
    let tracker = TrackerConfig::Linear {
        api_key:               "token".to_owned(),
        team_key:              "RAR".to_owned(),
        project_slug:          None,
        endpoint:              "https://api.linear.app/graphql".to_owned(),
        active_states:         vec!["Todo".to_owned()],
        terminal_states:       vec!["Done".to_owned()],
        repo_label_prefix:     "repo:".to_owned(),
        started_issue_state:   "In Dev".to_owned(),
        completed_issue_state: "QA".to_owned(),
    };

    assert_eq!(tracker.started_issue_state(), "In Dev");
    assert_eq!(tracker.completed_issue_state(), "QA");
}

#[test]
fn merge_core_config_overlays_core_fields_onto_generated_config() {
    let generated = "cli:\n  backend: codex\nevent_loop:\n  prompt_file: PROMPT.md\n";
    let core = "RObot:\n  enabled: true\n  timeout_seconds: 120\n";

    let merged = merge_core_config(generated, core).expect("merge should succeed");

    assert!(merged.contains("cli:"));
    assert!(merged.contains("backend: codex"));
    assert!(merged.contains("RObot:"));
    assert!(merged.contains("enabled: true"));
    assert!(merged.contains("timeout_seconds: 120"));
}
