//! Basic integration tests for rara-acp types and registry.
//!
//! These tests verify the registry, event types, and error types without
//! spawning a real ACP agent process.

use agent_client_protocol::{RequestPermissionOutcome, SelectedPermissionOutcome};
use rara_acp::{
    AcpEvent, AcpThreadStatus, AgentCommand, AgentKind, AgentRegistry, FileOperation,
    PermissionBridge, PermissionOptionInfo, StopReason, ToolCallStatus,
};
use tokio::sync::{mpsc, oneshot};

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
        AcpEvent::PermissionRequested {
            tool_call_id: "tc-2".into(),
            tool_title:   "write_file".into(),
            options:      vec![PermissionOptionInfo {
                id:    "allow".into(),
                label: "Allow".into(),
                kind:  "allow_once".into(),
            }],
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

// -- AcpThread + PermissionBridge tests --

#[tokio::test]
async fn permission_bridge_roundtrip() {
    let (perm_tx, mut perm_rx) = mpsc::channel::<PermissionBridge>(8);

    // Simulate a bridge arriving from the delegate.
    let (reply_tx, reply_rx) = oneshot::channel();
    perm_tx
        .send(PermissionBridge {
            tool_call_id: "tc-1".into(),
            tool_title: "Write auth.rs".into(),
            options: vec![
                PermissionOptionInfo {
                    id:    "allow".into(),
                    label: "Allow".into(),
                    kind:  "allow_once".into(),
                },
                PermissionOptionInfo {
                    id:    "deny".into(),
                    label: "Deny".into(),
                    kind:  "reject_once".into(),
                },
            ],
            reply_tx,
        })
        .await
        .unwrap();

    let bridge = perm_rx.recv().await.unwrap();
    assert_eq!(bridge.tool_title, "Write auth.rs");
    assert_eq!(bridge.tool_call_id, "tc-1");
    assert_eq!(bridge.options.len(), 2);

    // Simulate user approval.
    let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
        agent_client_protocol::PermissionOptionId::new("allow"),
    ));
    bridge.reply_tx.send(outcome).unwrap();

    let result = reply_rx.await.unwrap();
    assert!(matches!(result, RequestPermissionOutcome::Selected(_)));
}

#[tokio::test]
async fn dropped_reply_tx_yields_recv_error() {
    let (perm_tx, mut perm_rx) = mpsc::channel::<PermissionBridge>(8);

    let (_reply_tx, reply_rx) = oneshot::channel::<RequestPermissionOutcome>();
    perm_tx
        .send(PermissionBridge {
            tool_call_id: "tc-2".into(),
            tool_title:   "Delete data".into(),
            options:      vec![],
            reply_tx:     _reply_tx,
        })
        .await
        .unwrap();

    let bridge = perm_rx.recv().await.unwrap();
    // Drop the reply_tx — simulates handler crash or timeout.
    drop(bridge.reply_tx);

    // The original reply_rx should get a RecvError (channel closed).
    assert!(reply_rx.await.is_err());
}

#[test]
fn thread_status_transitions() {
    let status = AcpThreadStatus::Ready;
    assert!(matches!(status, AcpThreadStatus::Ready));

    let status = AcpThreadStatus::Generating;
    assert!(matches!(status, AcpThreadStatus::Generating));

    let status = AcpThreadStatus::WaitingForConfirmation {
        tool_call_id: "tc-1".into(),
        tool_title:   "Write file".into(),
        options:      vec![],
    };
    assert!(matches!(
        status,
        AcpThreadStatus::WaitingForConfirmation { .. }
    ));

    let status = AcpThreadStatus::TurnComplete {
        stop_reason: StopReason::EndTurn,
    };
    assert!(matches!(status, AcpThreadStatus::TurnComplete { .. }));

    let status = AcpThreadStatus::Disconnected;
    assert!(matches!(status, AcpThreadStatus::Disconnected));
}

#[test]
fn permission_option_info_is_clone_and_debug() {
    let opt = PermissionOptionInfo {
        id:    "allow-once".into(),
        label: "Allow Once".into(),
        kind:  "allow_once".into(),
    };
    let cloned = opt.clone();
    assert_eq!(cloned.id, "allow-once");
    assert!(!format!("{cloned:?}").is_empty());
}
