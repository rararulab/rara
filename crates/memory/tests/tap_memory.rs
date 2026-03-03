use std::str::FromStr;

use rara_memory::tape::{
    FileTapeStore, TapEntryKind, TapError, TapMemory, TapeService, current_tape,
    default_tape_context,
};
use serde_json::json;
use tempfile::tempdir;
use tokio::task::JoinSet;

#[tokio::test]
async fn store_supports_fork_merge_archive_and_listing() {
    let tempdir = tempdir().expect("tempdir");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let store = FileTapeStore::new(tempdir.path(), &workspace)
        .await
        .expect("store");
    let base = store
        .append(
            "session/main",
            TapEntryKind::Message,
            json!({"role": "user", "content": "remember alpha"}),
        )
        .await
        .expect("append base");
    assert_eq!(base.id, 1);

    let fork_name = store.fork("session/main").await.expect("fork");
    store
        .append(
            &fork_name,
            TapEntryKind::Event,
            json!({"name": "forked", "data": {"ok": true}}),
        )
        .await
        .expect("append fork");
    store
        .merge(&fork_name, "session/main")
        .await
        .expect("merge back");

    let names = store.list_tapes().await.expect("list tapes");
    assert!(names.contains(&"session/main".to_owned()));
    assert!(!names.contains(&fork_name));

    let merged = store
        .read("session/main")
        .await
        .expect("read")
        .expect("tape exists");
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[1].id, 2);

    let archived = store.archive("session/main").await.expect("archive");
    assert!(archived.is_some());
    assert!(
        store
            .read("session/main")
            .await
            .expect("read archived")
            .is_none()
    );
}

#[tokio::test]
async fn service_exposes_bub_style_lifecycle_queries_and_reset() {
    let tempdir = tempdir().expect("tempdir");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let store = FileTapeStore::new(tempdir.path(), &workspace)
        .await
        .expect("store");
    let service = TapeService::new("session/main", store);

    service.ensure_bootstrap_anchor().await.expect("bootstrap");
    service
        .append_system("system prompt")
        .await
        .expect("system");
    service
        .append_message(json!({"role": "user", "content": "hello world"}))
        .await
        .expect("message");
    service
        .append_tool_call(json!({
            "calls": [
                {
                    "id": "call-1",
                    "function": {"name": "search"}
                }
            ]
        }))
        .await
        .expect("tool call");
    service
        .append_tool_result(json!({"results": ["tool ok"]}))
        .await
        .expect("tool result");
    service
        .handoff("turn-1", Some(json!({"owner": "assistant"})))
        .await
        .expect("handoff");
    service
        .append_event("run", json!({"usage": {"total_tokens": 321}}))
        .await
        .expect("event");
    service
        .append_message(json!({"role": "assistant", "content": "after anchor"}))
        .await
        .expect("message after anchor");

    let info = service.info().await.expect("info");
    assert_eq!(info.name, "session/main");
    assert_eq!(info.anchors, 2);
    assert_eq!(info.last_anchor.as_deref(), Some("turn-1"));
    assert_eq!(info.entries_since_last_anchor, 2);
    assert_eq!(info.last_token_usage, Some(321));

    let anchors = service.anchors(10).await.expect("anchors");
    assert_eq!(anchors.len(), 2);
    assert_eq!(anchors[0].name, "session/start");
    assert_eq!(anchors[1].name, "turn-1");

    let after = service.after_anchor("turn-1", None).await.expect("after");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].kind, TapEntryKind::Event);

    let between = service
        .between_anchors("session/start", "turn-1", None)
        .await
        .expect("between");
    assert_eq!(between.len(), 4);

    let from_last = service.from_last_anchor(None).await.expect("from last");
    assert_eq!(from_last.len(), 3);
    assert_eq!(from_last[0].kind, TapEntryKind::Anchor);

    let search = service.search("hello", 10, false).await.expect("search");
    assert_eq!(search.len(), 1);
    assert_eq!(search[0].kind, TapEntryKind::Message);

    let fuzzy = service
        .search("helo", 10, false)
        .await
        .expect("fuzzy search");
    assert_eq!(fuzzy.len(), 1);
    assert_eq!(fuzzy[0].kind, TapEntryKind::Message);

    let archived = service.reset(true).await.expect("reset archive");
    assert!(archived.contains(".bak"));
    let reset_anchors = service.anchors(10).await.expect("anchors after reset");
    assert_eq!(reset_anchors.len(), 1);
    assert_eq!(reset_anchors[0].name, "session/start");
}

#[tokio::test]
async fn service_tracks_current_tape_during_fork_context() {
    let tempdir = tempdir().expect("tempdir");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let store = FileTapeStore::new(tempdir.path(), &workspace)
        .await
        .expect("store");
    let service = TapeService::new("session/main", store);
    service
        .append_message(json!({"role": "user", "content": "base"}))
        .await
        .expect("base append");

    assert_eq!(current_tape(), "-");

    let fork_name = service
        .fork_tape(|fork| async move {
            assert_eq!(current_tape(), fork.name());
            fork.append_message(json!({"role": "assistant", "content": "fork only"}))
                .await
                .expect("fork append");
            Ok(fork.name().to_owned())
        })
        .await
        .expect("fork tape");

    assert_ne!(fork_name, "session/main");
    assert_eq!(current_tape(), "-");

    let entries = service.entries().await.expect("entries");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].payload["content"], "fork only");
}

#[tokio::test]
async fn default_tape_context_reconstructs_message_sequence() {
    let tempdir = tempdir().expect("tempdir");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let store = FileTapeStore::new(tempdir.path(), &workspace)
        .await
        .expect("store");
    let tape = TapeService::new("session/main", store);

    tape.append_message(json!({"role": "user", "content": "hi"}))
        .await
        .expect("message");
    tape.append_tool_call(json!({
        "calls": [
            {
                "id": "call-1",
                "function": {"name": "search"}
            }
        ]
    }))
    .await
    .expect("tool call");
    tape.append_tool_result(json!({"results": ["ok"]}))
        .await
        .expect("tool result");

    let entries = tape.entries().await.expect("entries");
    let messages = default_tape_context(&entries).expect("context");

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[2]["role"], "tool");
    assert_eq!(messages[2]["tool_call_id"], "call-1");
    assert_eq!(messages[2]["name"], "search");
    assert_eq!(messages[2]["content"], "ok");
}

#[tokio::test]
async fn tape_uses_enum_derives_and_local_tape_errors() {
    let kind = TapEntryKind::from_str("tool_result").expect("enum parse");
    assert!(kind.is_tool_result());
    assert_eq!(kind.as_ref(), "tool_result");
    assert_eq!(kind.to_string(), "tool_result");

    let tempdir = tempdir().expect("tempdir");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let tape = TapMemory::new(tempdir.path(), &workspace, "session/main")
        .await
        .expect("tap");
    tape.append(
        TapEntryKind::Message,
        json!({"role": "user", "content": "seed"}),
    )
    .await
    .expect("seed tape file");
    let tape_dir = tempdir.path().join("tapes");
    let tape_file = std::fs::read_dir(&tape_dir)
        .expect("read tape dir")
        .next()
        .expect("tape file present")
        .expect("dir entry")
        .path();

    std::fs::write(&tape_file, "{not-json\n").expect("write invalid payload");
    let err = tape.entries().await.expect_err("expected tape json error");
    assert!(matches!(err, TapError::JsonDecode { .. }));
}

#[tokio::test]
async fn store_serializes_concurrent_append_only_writes() {
    let tempdir = tempdir().expect("tempdir");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let store = FileTapeStore::new(tempdir.path(), &workspace)
        .await
        .expect("store");

    let mut tasks = JoinSet::new();
    for index in 0..16 {
        let store = store.clone();
        tasks.spawn(async move {
            store
                .append(
                    "session/main",
                    TapEntryKind::Message,
                    json!({"role": "user", "content": format!("message-{index}")}),
                )
                .await
                .expect("append")
        });
    }

    while let Some(result) = tasks.join_next().await {
        result.expect("task finished");
    }

    let entries = store
        .read("session/main")
        .await
        .expect("read")
        .expect("tape exists");
    assert_eq!(entries.len(), 16);
    assert_eq!(
        entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        (1..=16).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn store_ignores_trailing_partial_json_line_until_it_is_completed() {
    let tempdir = tempdir().expect("tempdir");
    let workspace = tempdir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let store = FileTapeStore::new(tempdir.path(), &workspace)
        .await
        .expect("store");
    store
        .append(
            "session/main",
            TapEntryKind::Message,
            json!({"role": "user", "content": "seed"}),
        )
        .await
        .expect("seed tape file");

    let tape_dir = tempdir.path().join("tapes");
    let tape_file = std::fs::read_dir(&tape_dir)
        .expect("read tape dir")
        .next()
        .expect("tape file present")
        .expect("dir entry")
        .path();

    let seed_line = std::fs::read_to_string(&tape_file).expect("read tape file");
    let mut partial = String::new();
    partial.push_str(&seed_line);
    partial.push_str(r#"{"id":2,"kind":"message""#);
    std::fs::write(&tape_file, partial).expect("write trailing partial line");

    let first_read = store
        .read("session/main")
        .await
        .expect("read should ignore trailing partial line")
        .expect("tape exists");
    assert_eq!(first_read.len(), 1);
    assert_eq!(first_read[0].id, 1);

    let mut completed = std::fs::read_to_string(&tape_file).expect("read partial tape");
    completed.push_str(
        r#","payload":{"role":"assistant","content":"tail"},"timestamp":"2025-01-01T00:00:00Z"}"#,
    );
    completed.push('\n');
    std::fs::write(&tape_file, completed).expect("complete trailing line");

    let second_read = store
        .read("session/main")
        .await
        .expect("read completed line")
        .expect("tape exists");
    assert_eq!(second_read.len(), 2);
    assert_eq!(second_read[1].id, 2);
    assert_eq!(second_read[1].payload["content"], "tail");
}
