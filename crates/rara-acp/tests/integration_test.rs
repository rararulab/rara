//! Basic integration tests for rara-acp types and registry.
//!
//! These tests verify the registry, event types, and error types without
//! spawning a real ACP agent process.

use rara_acp::{
    AcpEvent, AgentCommand, AgentKind, AgentRegistry, FileOperation, StopReason, ToolCallStatus,
};

#[test]
fn registry_resolves_builtin_agents() {
    let registry = AgentRegistry::with_defaults();

    let claude = registry
        .resolve(&AgentKind::Claude)
        .expect("claude registered");
    assert_eq!(claude.program, "npx");
    assert!(claude.args.iter().any(|a| a.contains("claude")));

    let codex = registry
        .resolve(&AgentKind::Codex)
        .expect("codex registered");
    assert_eq!(codex.program, "npx");
    assert!(codex.args.iter().any(|a| a.contains("codex")));

    let gemini = registry
        .resolve(&AgentKind::Gemini)
        .expect("gemini registered");
    assert_eq!(gemini.program, "gemini");
}

#[test]
fn registry_custom_agent() {
    let mut registry = AgentRegistry::with_defaults();

    let kind = AgentKind::Custom("my-agent".into());
    let cmd = AgentCommand {
        program: "my-agent".into(),
        args:    vec!["--acp".into()],
        env:     vec![],
    };
    registry.register(kind.clone(), cmd);

    let resolved = registry.resolve(&kind).expect("custom agent registered");
    assert_eq!(resolved.program, "my-agent");
    assert_eq!(resolved.args, vec!["--acp"]);
}

#[test]
fn registry_unknown_custom_returns_none() {
    let registry = AgentRegistry::with_defaults();
    let result = registry.resolve(&AgentKind::Custom("nonexistent".into()));
    assert!(result.is_none());
}

#[test]
fn event_types_are_clone_and_debug() {
    let events: Vec<AcpEvent> = vec![
        AcpEvent::Thinking("reasoning".into()),
        AcpEvent::Text("hello".into()),
        AcpEvent::ToolCallStarted {
            id:    "tc-1".into(),
            title: "read_file".into(),
        },
        AcpEvent::ToolCallUpdate {
            id:     "tc-1".into(),
            status: ToolCallStatus::Completed,
            output: Some("done".into()),
        },
        AcpEvent::Plan {
            title: Some("plan".into()),
            steps: vec!["step 1".into()],
        },
        AcpEvent::TurnComplete {
            stop_reason: StopReason::EndTurn,
        },
        AcpEvent::ProcessExited { code: Some(0) },
        AcpEvent::PermissionAutoApproved {
            description: "approved".into(),
        },
        AcpEvent::FileAccess {
            path:      "/tmp/test.rs".into(),
            operation: FileOperation::Read,
        },
    ];

    for event in &events {
        let cloned = event.clone();
        let debug = format!("{cloned:?}");
        assert!(!debug.is_empty());
    }
}

#[test]
fn stop_reason_equality() {
    assert_eq!(StopReason::EndTurn, StopReason::EndTurn);
    assert_eq!(StopReason::Cancelled, StopReason::Cancelled);
    assert_ne!(StopReason::EndTurn, StopReason::Cancelled);
    assert_eq!(
        StopReason::Error("oops".into()),
        StopReason::Error("oops".into())
    );
}

#[test]
fn tool_call_status_equality() {
    assert_eq!(ToolCallStatus::Running, ToolCallStatus::Running);
    assert_eq!(ToolCallStatus::Completed, ToolCallStatus::Completed);
    assert_eq!(ToolCallStatus::Failed, ToolCallStatus::Failed);
    assert_ne!(ToolCallStatus::Running, ToolCallStatus::Failed);
}
