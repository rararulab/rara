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

//! Integration tests for the cross-session topology WS endpoint
//! (`/topology/{root_session_key}`) introduced in #1999.
//!
//! The two scenarios bound by the spec:
//!
//! 1. Subscribe to the root, then a `SubagentSpawned` for child `c1` arrives on
//!    the root's bus. The handler must dynamically subscribe to `c1`'s bus and
//!    forward subsequent `c1` events on the same socket.
//! 2. Subscribe to the root, then a chain of spawns root → c1 → c2. Events on
//!    the grandchild `c2`'s bus must reach the socket too.
//!
//! Both tests boot a real kernel via [`TestKernelBuilder`] so the
//! `WebAdapter` can be `start()`ed (the topology handler reads
//! `state.stream_hub`, populated only after `start`). Topology events are
//! injected directly via [`StreamHub::emit_to_session_bus`] — the same
//! kernel-internal API the production code uses — to avoid coupling the
//! test to the agent loop's spawn/done timing.

use std::{
    path::PathBuf,
    sync::{Arc, Once},
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use rara_channels::{
    web::WebAdapter, web_reply_buffer::ReplyBuffer,
    web_session::set_session_ws_keepalive_interval_for_tests, web_topology::TopologyFrame,
};
use rara_kernel::{
    channel::adapter::ChannelAdapter,
    io::StreamEvent,
    session::SessionKey,
    testing::{TestKernelBuilder, scripted_response},
};
use tokio_tungstenite::tungstenite::Message;

const OWNER_TOKEN: &str = "test-owner-token";
const OWNER_USER_ID: &str = "test-user";

fn init_test_env() {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    static INIT: Once = Once::new();
    let root = ROOT.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("rara-ws-topology-test-{}", std::process::id()));
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
    static KEEPALIVE: Once = Once::new();
    KEEPALIVE.call_once(|| {
        // Match other channels tests so the global keepalive interval is
        // sub-second; the topology endpoint inherits the same WS plumbing.
        set_session_ws_keepalive_interval_for_tests(Some(Duration::from_millis(200)));
    });
}

fn topology_url(addr: std::net::SocketAddr, root: &SessionKey, token: &str) -> String {
    format!("ws://{addr}/chat/topology/{root}?token={token}")
}

async fn boot_with_kernel() -> (
    std::net::SocketAddr,
    Arc<WebAdapter>,
    rara_kernel::testing::TestKernel,
    tokio::task::JoinHandle<()>,
) {
    init_test_env();
    let tmp = tempfile::tempdir().expect("tempdir");
    // Box::leak the tempdir guard so the on-disk state lives for the
    // duration of the test process (matches `web_session_smoke` pattern
    // for tests that need a live kernel without juggling a guard).
    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![scripted_response("ok")])
        .build()
        .await;
    // Forget the tempdir — the test process exits long before this matters.
    Box::leak(Box::new(tmp));

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
    (addr, adapter, tk, server)
}

/// Read the next `TopologyFrame` from the WS, ignoring transport-level
/// frames (Ping / Pong).
async fn next_frame<S>(stream: &mut S) -> TopologyFrame
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
            Message::Text(t) => {
                return serde_json::from_str(t.as_str()).expect("TopologyFrame JSON");
            }
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => panic!("server closed unexpectedly"),
            Message::Frame(_) => continue,
        }
    }
}

/// Wait for the first frame matching `pred`, draining everything before it.
/// Returns `None` on a 5 s timeout.
async fn wait_for_frame<F, S>(stream: &mut S, mut pred: F) -> Option<TopologyFrame>
where
    F: FnMut(&TopologyFrame) -> bool,
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let frame_res = tokio::time::timeout(remaining, stream.next()).await;
        let frame = match frame_res {
            Ok(Some(Ok(m))) => m,
            _ => return None,
        };
        let Message::Text(t) = frame else { continue };
        let parsed: TopologyFrame = serde_json::from_str(t.as_str()).expect("TopologyFrame JSON");
        if pred(&parsed) {
            return Some(parsed);
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario 1: spawn arriving on the root's bus grows the watch set
// ---------------------------------------------------------------------------

/// Subscribing to the root, then emitting `SubagentSpawned` on the root's
/// bus, must cause the handler to dynamically add the child to the watch
/// set and forward subsequent events on the child's bus over the same
/// socket — which is the entire point of `/topology/{root}`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_root_then_spawn_child_delivers_child_events() {
    let (addr, _adapter, tk, server) = boot_with_kernel().await;

    let root = SessionKey::new();
    let child = SessionKey::new();

    let (mut ws, _resp) = tokio_tungstenite::connect_async(&topology_url(addr, &root, OWNER_TOKEN))
        .await
        .expect("ws connect");

    // Hello first.
    let hello = next_frame(&mut ws).await;
    match hello {
        TopologyFrame::Hello {
            ref root_session_key,
            ref initial_descendants,
        } => {
            assert_eq!(root_session_key, &root.to_string());
            assert!(
                initial_descendants.is_empty(),
                "root with no spawned children should snapshot empty, got {initial_descendants:?}"
            );
        }
        other => panic!("expected Hello, got {other:?}"),
    }

    // Then a SessionSubscribed for the root itself (snapshot subscribe
    // path emits one before its forwarder starts). May be interleaved
    // ahead of the spawn event we inject below; just drain until we see
    // it, then continue.
    let subscribed = wait_for_frame(&mut ws, |f| {
        matches!(f, TopologyFrame::SessionSubscribed { session_key, .. } if session_key == &root.to_string())
    })
    .await
    .expect("SessionSubscribed for root within 5s");
    let _ = subscribed;

    // Inject SubagentSpawned on root's bus — production code path is
    // identical (kernel calls emit_to_session_bus from handle_spawn_agent).
    tk.handle.stream_hub().emit_to_session_bus(
        &root,
        StreamEvent::SubagentSpawned {
            parent_session: root,
            child_session:  child,
            manifest_name:  "researcher".to_owned(),
        },
    );

    // Expect: SessionSubscribed{child} AND Event{root, SubagentSpawned}.
    // Order between them: handler emits SessionSubscribed inline before
    // forwarding the spawn event itself.
    let mut got_subscribed = false;
    let mut got_event = false;
    while !(got_subscribed && got_event) {
        let frame = wait_for_frame(&mut ws, |_| true)
            .await
            .expect("subscribed+event within 5s");
        match frame {
            TopologyFrame::SessionSubscribed {
                session_key,
                parent,
            } if session_key == child.to_string() => {
                assert_eq!(parent.as_deref(), Some(root.to_string().as_str()));
                got_subscribed = true;
            }
            TopologyFrame::Event { session_key, event } if session_key == root.to_string() => {
                use rara_channels::web::WebEvent;
                if let WebEvent::SubagentSpawned {
                    parent_session,
                    child_session,
                    manifest_name,
                } = event
                {
                    assert_eq!(parent_session, root.to_string());
                    assert_eq!(child_session, child.to_string());
                    assert_eq!(manifest_name, "researcher");
                    got_event = true;
                }
            }
            _ => continue,
        }
    }

    // Now emit a Progress event on the *child*'s bus — this is the
    // load-bearing assertion: without dynamic subscription the handler
    // would never see it.
    tk.handle.stream_hub().emit_to_session_bus(
        &child,
        StreamEvent::Progress {
            stage: "fetching".to_owned(),
        },
    );
    let frame = wait_for_frame(&mut ws, |f| {
        matches!(
            f,
            TopologyFrame::Event { session_key, event: rara_channels::web::WebEvent::Progress { stage } }
                if session_key == &child.to_string() && stage == "fetching"
        )
    })
    .await
    .expect("child Progress event within 5s");
    let _ = frame;

    ws.send(Message::Close(None)).await.ok();
    drop(ws);
    server.abort();
    tk.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 2: chain of spawns root → c1 → c2 — grandchild events reach the WS
// ---------------------------------------------------------------------------

/// A chain of spawns must transitively grow the watch set: emit a spawn
/// on root's bus naming `c1`, then a spawn on `c1`'s bus naming `c2`, and
/// the socket must observe a `Progress` on `c2`'s bus.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grandchild_via_chain_of_spawns_is_followed() {
    let (addr, _adapter, tk, server) = boot_with_kernel().await;

    let root = SessionKey::new();
    let c1 = SessionKey::new();
    let c2 = SessionKey::new();

    let (mut ws, _resp) = tokio_tungstenite::connect_async(&topology_url(addr, &root, OWNER_TOKEN))
        .await
        .expect("ws connect");

    // Drain Hello + root SessionSubscribed.
    let _ = next_frame(&mut ws).await;
    let _ = wait_for_frame(&mut ws, |f| {
        matches!(f, TopologyFrame::SessionSubscribed { session_key, .. } if session_key == &root.to_string())
    })
    .await
    .expect("root SessionSubscribed");

    // Spawn c1 on root.
    tk.handle.stream_hub().emit_to_session_bus(
        &root,
        StreamEvent::SubagentSpawned {
            parent_session: root,
            child_session:  c1,
            manifest_name:  "stage-1".to_owned(),
        },
    );
    let _ = wait_for_frame(&mut ws, |f| {
        matches!(f, TopologyFrame::SessionSubscribed { session_key, .. } if session_key == &c1.to_string())
    })
    .await
    .expect("c1 SessionSubscribed");

    // Spawn c2 on c1 — this exercises the recursive grow path.
    tk.handle.stream_hub().emit_to_session_bus(
        &c1,
        StreamEvent::SubagentSpawned {
            parent_session: c1,
            child_session:  c2,
            manifest_name:  "stage-2".to_owned(),
        },
    );
    let _ = wait_for_frame(&mut ws, |f| {
        matches!(f, TopologyFrame::SessionSubscribed { session_key, .. } if session_key == &c2.to_string())
    })
    .await
    .expect("c2 SessionSubscribed within 5s");

    // Now emit on c2 and verify the socket sees it.
    tk.handle.stream_hub().emit_to_session_bus(
        &c2,
        StreamEvent::Progress {
            stage: "deep".to_owned(),
        },
    );
    let frame = wait_for_frame(&mut ws, |f| {
        matches!(
            f,
            TopologyFrame::Event { session_key, event: rara_channels::web::WebEvent::Progress { stage } }
                if session_key == &c2.to_string() && stage == "deep"
        )
    })
    .await
    .expect("c2 Progress within 5s");
    let _ = frame;

    ws.send(Message::Close(None)).await.ok();
    drop(ws);
    server.abort();
    tk.shutdown();
}
