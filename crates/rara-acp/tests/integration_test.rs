//! Integration tests for rara-acp types and FSAcpRegistry.
//!
//! These tests verify the registry, event types, and error types without
//! spawning a real ACP agent process.

use agent_client_protocol::{RequestPermissionOutcome, SelectedPermissionOutcome};
use rara_acp::{
    AcpAgentConfig, AcpEvent, AcpRegistry, AcpThreadStatus, FSAcpRegistry, FileOperation,
    PermissionBridge, PermissionOptionInfo, StopReason, ToolCallStatus,
};
use tokio::sync::{mpsc, oneshot};

// -- FSAcpRegistry tests --

#[tokio::test]
async fn fs_registry_load_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-agents.json");
    let registry = FSAcpRegistry::load(&path).await.unwrap();
    let agents = registry.list().await.unwrap();
    assert!(agents.is_empty());
}

#[tokio::test]
async fn fs_registry_add_and_get() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-agents.json");
    let registry = FSAcpRegistry::load(&path).await.unwrap();

    let config = AcpAgentConfig {
        command: "my-agent".into(),
        args: vec!["--acp".into()],
        enabled: true,
        ..Default::default()
    };
    registry
        .add("test-agent".into(), config.clone())
        .await
        .unwrap();

    let retrieved = registry.get("test-agent").await.unwrap().unwrap();
    assert_eq!(retrieved.command, "my-agent");
    assert_eq!(retrieved.args, vec!["--acp"]);
    assert!(retrieved.enabled);
    assert!(!retrieved.builtin);
}

#[tokio::test]
async fn fs_registry_remove_custom() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-agents.json");
    let registry = FSAcpRegistry::load(&path).await.unwrap();

    let config = AcpAgentConfig {
        command: "test".into(),
        enabled: true,
        ..Default::default()
    };
    registry.add("removable".into(), config).await.unwrap();
    assert!(registry.remove("removable").await.unwrap());
    assert!(registry.get("removable").await.unwrap().is_none());
}

#[tokio::test]
async fn fs_registry_cannot_remove_builtin() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-agents.json");
    let registry = FSAcpRegistry::load(&path).await.unwrap();

    let config = AcpAgentConfig {
        command: "npx".into(),
        enabled: true,
        builtin: true,
        ..Default::default()
    };
    registry.add("claude".into(), config).await.unwrap();
    let err = registry.remove("claude").await.unwrap_err();
    assert!(err.to_string().contains("cannot remove builtin"));
}

#[tokio::test]
async fn fs_registry_cannot_disable_builtin() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-agents.json");
    let registry = FSAcpRegistry::load(&path).await.unwrap();

    let config = AcpAgentConfig {
        command: "npx".into(),
        enabled: true,
        builtin: true,
        ..Default::default()
    };
    registry.add("claude".into(), config).await.unwrap();
    let err = registry.disable("claude").await.unwrap_err();
    assert!(err.to_string().contains("cannot disable builtin"));
}

#[tokio::test]
async fn fs_registry_cannot_overwrite_builtin_with_non_builtin() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-agents.json");
    let registry = FSAcpRegistry::load(&path).await.unwrap();

    let builtin = AcpAgentConfig {
        command: "npx".into(),
        enabled: true,
        builtin: true,
        ..Default::default()
    };
    registry.add("claude".into(), builtin).await.unwrap();

    // A non-builtin config should not overwrite the builtin.
    let non_builtin = AcpAgentConfig {
        command: "my-claude".into(),
        enabled: true,
        builtin: false,
        ..Default::default()
    };
    let err = registry
        .add("claude".into(), non_builtin)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("cannot overwrite builtin"));

    // The original config should be unchanged.
    let retrieved = registry.get("claude").await.unwrap().unwrap();
    assert_eq!(retrieved.command, "npx");
    assert!(retrieved.builtin);
}

#[tokio::test]
async fn fs_registry_enable_disable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-agents.json");
    let registry = FSAcpRegistry::load(&path).await.unwrap();

    let config = AcpAgentConfig {
        command: "test".into(),
        enabled: true,
        ..Default::default()
    };
    registry.add("toggleable".into(), config).await.unwrap();

    assert!(registry.disable("toggleable").await.unwrap());
    let disabled = registry.get("toggleable").await.unwrap().unwrap();
    assert!(!disabled.enabled);

    assert!(registry.enable("toggleable").await.unwrap());
    let enabled = registry.get("toggleable").await.unwrap().unwrap();
    assert!(enabled.enabled);
}

#[tokio::test]
async fn fs_registry_enabled_agents_filter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-agents.json");
    let registry = FSAcpRegistry::load(&path).await.unwrap();

    registry
        .add(
            "enabled-one".into(),
            AcpAgentConfig {
                command: "a".into(),
                enabled: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    registry
        .add(
            "disabled-one".into(),
            AcpAgentConfig {
                command: "b".into(),
                enabled: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let enabled = registry.enabled_agents().await.unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].0, "enabled-one");
}

#[tokio::test]
async fn fs_registry_persists_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("acp-agents.json");

    // Write
    {
        let registry = FSAcpRegistry::load(&path).await.unwrap();
        registry
            .add(
                "persistent".into(),
                AcpAgentConfig {
                    command: "test".into(),
                    enabled: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    // Read back from disk
    let registry = FSAcpRegistry::load(&path).await.unwrap();
    let agent = registry.get("persistent").await.unwrap();
    assert!(agent.is_some());
    assert_eq!(agent.unwrap().command, "test");
}

#[tokio::test]
async fn fs_registry_to_agent_command() {
    let config = AcpAgentConfig {
        command: "npx".into(),
        args: vec!["-y".into(), "some-pkg".into()],
        enabled: true,
        ..Default::default()
    };
    let cmd = config.to_agent_command();
    assert_eq!(cmd.program, "npx");
    assert_eq!(cmd.args, vec!["-y", "some-pkg"]);
    assert!(cmd.env.is_empty());
}

// -- Event type tests --

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
