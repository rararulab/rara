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

//! Integration tests for the persistent per-session WS endpoint
//! (`/session/{session_key}`) introduced in #1935 phase (a).
//!
//! Each test boots a real `axum` server backed by `WebAdapter::router()`
//! and connects a `tokio_tungstenite` client over a TCP loopback. Tests
//! that exercise the kernel notification bus boot a real kernel via
//! [`TestKernelBuilder`]; tests that only exercise the adapter-local bus
//! drive the adapter directly without starting the kernel (matches the
//! pattern used by `web_ws_drain.rs`).

use std::{
    path::PathBuf,
    sync::{Arc, Once},
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use rara_channels::{
    web::{WebAdapter, WebEvent},
    web_reply_buffer::ReplyBuffer,
};
use rara_kernel::{
    channel::{adapter::ChannelAdapter, types::ChannelType},
    io::{Endpoint, EndpointAddress, PlatformOutbound},
    notification::KernelNotification,
    session::SessionKey,
};
use tokio_tungstenite::tungstenite::Message;

const OWNER_TOKEN: &str = "test-owner-token";
const OWNER_USER_ID: &str = "test-user";

/// Override `rara_paths` to a stable per-process temp dir so kernel-backed
/// tests don't touch `~/.config/rara`.
fn init_test_env() {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    static INIT: Once = Once::new();
    let root = ROOT.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("rara-ws-session-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create test env root");
        dir
    });
    INIT.call_once(move || {
        let data = root.join("rara_data");
        let config = root.join("rara_config");
        std::fs::create_dir_all(&data).expect("create test data dir");
        std::fs::create_dir_all(&config).expect("create test config dir");
        rara_paths::set_custom_data_dir(&data);
        rara_paths::set_custom_config_dir(&config);
    });
}

fn web_endpoint(session_key: &SessionKey) -> Endpoint {
    Endpoint {
        channel_type: ChannelType::Web,
        address:      EndpointAddress::Web {
            connection_id: session_key.to_string(),
        },
    }
}

/// Read the next `WebEvent` from the WS stream, ignoring transport-level
/// frames (Ping / Pong).
async fn next_event<S>(stream: &mut S) -> WebEvent
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("ws frame within timeout")
            .expect("stream not closed")
            .expect("ws frame ok");
        match frame {
            Message::Text(t) => return serde_json::from_str(t.as_str()).expect("WebEvent JSON"),
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => panic!("server closed unexpectedly"),
            Message::Frame(_) => continue,
        }
    }
}

/// Boot the adapter under an axum loopback server, returning the bound
/// address, the adapter, and the server task handle.
async fn boot_adapter(
    buffer: Arc<ReplyBuffer>,
) -> (
    std::net::SocketAddr,
    Arc<WebAdapter>,
    tokio::task::JoinHandle<()>,
) {
    let adapter = Arc::new(
        WebAdapter::new(OWNER_TOKEN.to_owned(), OWNER_USER_ID.to_owned())
            .with_reply_buffer(Arc::clone(&buffer)),
    );
    let app = axum::Router::new().nest("/chat", adapter.router());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local_addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (addr, adapter, server)
}

fn session_url(addr: std::net::SocketAddr, key: &SessionKey, token: &str) -> String {
    format!("ws://{addr}/chat/session/{key}?token={token}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// On connect, the very first frame must be `hello` so the client knows
/// the socket is established. This anchors the contract in
/// `crate::web::WebEvent::Hello` and the in-line documentation about the
/// frame ordering invariant.
#[tokio::test]
async fn session_ws_emits_hello_on_connect() {
    let buffer = ReplyBuffer::new();
    let (addr, _adapter, server) = boot_adapter(Arc::clone(&buffer)).await;

    let session_key = SessionKey::new();
    let (mut ws, _resp) =
        tokio_tungstenite::connect_async(&session_url(addr, &session_key, OWNER_TOKEN))
            .await
            .expect("ws connect");

    let first = next_event(&mut ws).await;
    assert!(
        matches!(first, WebEvent::Hello),
        "first frame must be Hello, got {first:?}"
    );

    ws.send(Message::Close(None)).await.ok();
    drop(ws);
    server.abort();
}

/// Buffered events that fired while no listener was attached must drain
/// after `hello` and before any live forwarder events. This is the
/// #1804 invariant carried over to the new persistent WS endpoint.
#[tokio::test]
async fn session_ws_drains_backlog_after_hello() {
    let buffer = ReplyBuffer::new();
    let (addr, adapter, server) = boot_adapter(Arc::clone(&buffer)).await;

    let session_key = SessionKey::new();
    let endpoint = web_endpoint(&session_key);

    // Publish two important events while no listener exists. They land
    // in the per-session ReplyBuffer (no-receivers branch).
    adapter
        .send(
            &endpoint,
            PlatformOutbound::Reply {
                content:       "buffered-1".to_owned(),
                attachments:   Vec::new(),
                reply_context: None,
            },
        )
        .await
        .expect("egress send #1");
    adapter
        .send(
            &endpoint,
            PlatformOutbound::Reply {
                content:       "buffered-2".to_owned(),
                attachments:   Vec::new(),
                reply_context: None,
            },
        )
        .await
        .expect("egress send #2");
    assert_eq!(buffer.snapshot(&session_key).len(), 2);

    let (mut ws, _resp) =
        tokio_tungstenite::connect_async(&session_url(addr, &session_key, OWNER_TOKEN))
            .await
            .expect("ws connect");

    // First three frames: Hello, then the two buffered messages in order.
    let f1 = next_event(&mut ws).await;
    assert!(matches!(f1, WebEvent::Hello), "expected Hello, got {f1:?}");

    let f2 = next_event(&mut ws).await;
    match f2 {
        WebEvent::Message { content } => assert_eq!(content, "buffered-1"),
        other => panic!("expected buffered-1, got {other:?}"),
    }
    let f3 = next_event(&mut ws).await;
    match f3 {
        WebEvent::Message { content } => assert_eq!(content, "buffered-2"),
        other => panic!("expected buffered-2, got {other:?}"),
    }

    // A live publish after connect must arrive via the adapter forwarder.
    adapter
        .send(
            &endpoint,
            PlatformOutbound::Reply {
                content:       "live-after".to_owned(),
                attachments:   Vec::new(),
                reply_context: None,
            },
        )
        .await
        .expect("egress send live");

    let f4 = next_event(&mut ws).await;
    match f4 {
        WebEvent::Message { content } => assert_eq!(content, "live-after"),
        other => panic!("expected live-after, got {other:?}"),
    }

    ws.send(Message::Close(None)).await.ok();
    drop(ws);
    server.abort();
}

/// A wrong owner token must reject the upgrade with 401, matching the
/// legacy chat WS auth behavior.
#[tokio::test]
async fn session_ws_rejects_invalid_owner_token() {
    let buffer = ReplyBuffer::new();
    let (addr, _adapter, server) = boot_adapter(Arc::clone(&buffer)).await;

    let session_key = SessionKey::new();
    let url = format!("ws://{addr}/chat/session/{session_key}?token=wrong");
    let result = tokio_tungstenite::connect_async(&url).await;
    assert!(result.is_err(), "wrong token must reject the upgrade");

    server.abort();
}

/// During phase (a), an inbound `prompt` frame must be rejected with a
/// loud error frame so a half-built endpoint cannot accidentally serve
/// traffic. This guards against accidentally enabling the new endpoint
/// before phase (b) wires the full inbound path.
#[tokio::test]
async fn session_ws_rejects_prompt_during_phase_a() {
    let buffer = ReplyBuffer::new();
    let (addr, _adapter, server) = boot_adapter(Arc::clone(&buffer)).await;

    let session_key = SessionKey::new();
    let (mut ws, _resp) =
        tokio_tungstenite::connect_async(&session_url(addr, &session_key, OWNER_TOKEN))
            .await
            .expect("ws connect");

    // Drain the initial `hello`.
    let _ = next_event(&mut ws).await;

    let prompt = serde_json::json!({
        "type": "prompt",
        "content": "hello server",
    })
    .to_string();
    ws.send(Message::Text(prompt.into()))
        .await
        .expect("send prompt");

    let response = next_event(&mut ws).await;
    match response {
        WebEvent::Error { message } => {
            assert!(
                message.contains("phase b"),
                "expected phase-b mention in error, got: {message}"
            );
        }
        other => panic!("expected Error frame rejecting prompt, got {other:?}"),
    }

    ws.send(Message::Close(None)).await.ok();
    drop(ws);
    server.abort();
}

/// An inbound `abort` frame must be parsed without producing an error.
/// Phase (a) is a logged no-op; this test confirms the parser accepts
/// the contract so phase (b) can drop in real handling without changing
/// the wire format. We assert by sending an abort then a prompt and
/// confirming only the prompt produces an error frame.
#[tokio::test]
async fn session_ws_parses_abort_without_error() {
    let buffer = ReplyBuffer::new();
    let (addr, _adapter, server) = boot_adapter(Arc::clone(&buffer)).await;

    let session_key = SessionKey::new();
    let (mut ws, _resp) =
        tokio_tungstenite::connect_async(&session_url(addr, &session_key, OWNER_TOKEN))
            .await
            .expect("ws connect");

    let _ = next_event(&mut ws).await; // hello

    ws.send(Message::Text("{\"type\":\"abort\"}".into()))
        .await
        .expect("send abort");

    // Now send a prompt to elicit a single error frame; if abort had also
    // produced an error we would observe two error frames here.
    ws.send(Message::Text(
        serde_json::json!({"type": "prompt", "content": "x"})
            .to_string()
            .into(),
    ))
    .await
    .expect("send prompt");

    let response = next_event(&mut ws).await;
    assert!(
        matches!(response, WebEvent::Error { .. }),
        "expected exactly one Error frame from prompt, got {response:?}"
    );

    // No further frames should arrive in a short window — confirms abort
    // did not also emit anything.
    let timed = tokio::time::timeout(Duration::from_millis(200), ws.next()).await;
    assert!(
        timed.is_err(),
        "abort must be a phase-a no-op; instead got: {timed:?}"
    );

    ws.send(Message::Close(None)).await.ok();
    drop(ws);
    server.abort();
}

/// A `KernelNotification::TapeAppended` published by the kernel for the
/// connected session must arrive on the WS as a `tape_appended` frame.
/// This validates the third forwarder (notification bus → mpsc) that
/// the persistent endpoint adds on top of the legacy two-source merge.
///
/// We bypass the user-turn path and publish directly on the kernel's
/// notification bus, which mirrors the kernel-internal call site in
/// `crates/kernel/src/memory/service.rs` after a tape write.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_ws_forwards_tape_appended_from_notification_bus() {
    use rara_kernel::testing::TestKernelBuilder;

    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();

    let tk = TestKernelBuilder::new(tmp.path()).build().await;

    let buffer = ReplyBuffer::new();
    let adapter = Arc::new(
        WebAdapter::new(OWNER_TOKEN.to_owned(), OWNER_USER_ID.to_owned())
            .with_reply_buffer(Arc::clone(&buffer)),
    );
    adapter
        .start(tk.handle.clone())
        .await
        .expect("adapter start");

    let app = axum::Router::new().nest("/chat", adapter.router());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local_addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });

    let session_key = SessionKey::new();
    let (mut ws, _resp) =
        tokio_tungstenite::connect_async(&session_url(addr, &session_key, OWNER_TOKEN))
            .await
            .expect("ws connect");

    let _ = next_event(&mut ws).await; // hello

    // Give the notification forwarder a moment to attach its
    // subscription before publishing — broadcast subscriptions started
    // *after* a publish miss it.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let timestamp = jiff::Timestamp::now();
    tk.handle
        .notification_bus()
        .publish(KernelNotification::TapeAppended {
            session_key,
            entry_id: 42,
            role: Some("assistant".to_owned()),
            timestamp,
        })
        .await;

    let frame = next_event(&mut ws).await;
    match frame {
        WebEvent::TapeAppended {
            entry_id,
            role,
            timestamp: ts,
        } => {
            assert_eq!(entry_id, 42);
            assert_eq!(role.as_deref(), Some("assistant"));
            assert_eq!(ts, timestamp.to_string());
        }
        other => panic!("expected TapeAppended frame, got {other:?}"),
    }

    // A TapeAppended for a *different* session must not leak into this
    // socket — the per-session filter is what enforces #1867 isolation.
    let other_key = SessionKey::new();
    tk.handle
        .notification_bus()
        .publish(KernelNotification::TapeAppended {
            session_key: other_key,
            entry_id:    99,
            role:        Some("user".to_owned()),
            timestamp:   jiff::Timestamp::now(),
        })
        .await;
    let timed = tokio::time::timeout(Duration::from_millis(200), ws.next()).await;
    assert!(
        timed.is_err(),
        "cross-session TapeAppended must be filtered, got: {timed:?}"
    );

    ws.send(Message::Close(None)).await.ok();
    drop(ws);
    server.abort();
    tk.shutdown();
}
