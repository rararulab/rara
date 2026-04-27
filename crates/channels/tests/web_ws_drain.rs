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

//! Integration test for the WS handler's "subscribe → drain backlog →
//! forwarder" sequencing invariant (issue #1804).
//!
//! Boots a real `axum` server backed by `WebAdapter::router()`, connects a
//! `tokio_tungstenite` client over a TCP loopback, and asserts:
//!
//! 1. An event published while no listener is attached lands in the per-session
//!    `ReplyBuffer`.
//! 2. On WS connect the client receives the buffered backlog **before** any
//!    live event published after the upgrade.
//!
//! This is the race-free guarantee the WS handler must uphold: the snapshot
//! drain runs against the *already-subscribed* adapter bus, so a publish that
//! arrives between subscribe and drain is captured by the broadcast forwarder
//! (and dedupable later) — but never lost.

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use rara_channels::{
    web::{WebAdapter, WebEvent},
    web_reply_buffer::ReplyBuffer,
};
use rara_kernel::{
    channel::{adapter::ChannelAdapter, types::ChannelType},
    io::{Endpoint, EndpointAddress, PlatformOutbound},
    session::SessionKey,
};
use tokio_tungstenite::tungstenite::Message;

const OWNER_TOKEN: &str = "test-owner-token";
const OWNER_USER_ID: &str = "test-user";

fn web_endpoint(session_key: &SessionKey) -> Endpoint {
    Endpoint {
        channel_type: ChannelType::Web,
        address:      EndpointAddress::Web {
            connection_id: session_key.to_string(),
        },
    }
}

/// Read the next `WebEvent` from the WS stream, ignoring transport-level
/// frames (Ping / Pong / Close).
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
            Message::Text(t) => {
                return serde_json::from_str(t.as_str()).expect("WebEvent JSON");
            }
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => panic!("server closed unexpectedly"),
            Message::Frame(_) => continue,
        }
    }
}

#[tokio::test]
async fn ws_drains_backlog_before_live_events() {
    let buffer = ReplyBuffer::new();
    let adapter = Arc::new(
        WebAdapter::new(OWNER_TOKEN.to_owned(), OWNER_USER_ID.to_owned())
            .with_reply_buffer(Arc::clone(&buffer)),
    );

    // Mount the adapter under /chat to mirror production wiring.
    let app = axum::Router::new().nest("/chat", adapter.router());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local_addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });

    let session_key = SessionKey::new();
    let endpoint = web_endpoint(&session_key);

    // Step 1: publish a buffered event while no WS listener is attached.
    // This lands in the ReplyBuffer (the no-listeners branch in
    // `publish_adapter_event`).
    adapter
        .send(
            &endpoint,
            PlatformOutbound::Reply {
                content:       "buffered-while-away".to_owned(),
                attachments:   Vec::new(),
                reply_context: None,
            },
        )
        .await
        .expect("egress send #1");

    // Sanity: buffer captured the event.
    assert_eq!(buffer.snapshot(&session_key).len(), 1);

    // Step 2: connect a real WS client.
    let url = format!(
        "ws://{addr}/chat/ws?session_key={key}&token={tok}",
        key = session_key,
        tok = OWNER_TOKEN
    );
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("ws connect");

    // Step 3: the very first frame must be the buffered "while-away" reply,
    // not anything that follows. This proves drain happens before live
    // forwarding.
    let first = next_event(&mut ws).await;
    match first {
        WebEvent::Message { content } => {
            assert_eq!(content, "buffered-while-away", "drain must fire first");
        }
        other => panic!("expected backlog Message first, got {other:?}"),
    }

    // Step 4: now publish a live event. It must arrive via the broadcast
    // forwarder.
    adapter
        .send(
            &endpoint,
            PlatformOutbound::Reply {
                content:       "live-after-connect".to_owned(),
                attachments:   Vec::new(),
                reply_context: None,
            },
        )
        .await
        .expect("egress send #2");

    let second = next_event(&mut ws).await;
    match second {
        WebEvent::Message { content } => assert_eq!(content, "live-after-connect"),
        other => panic!("expected live Message second, got {other:?}"),
    }

    // Tidy up: client closes, server task is aborted at scope end.
    ws.send(Message::Close(None)).await.ok();
    drop(ws);
    server.abort();
}

#[tokio::test]
async fn ws_rejects_invalid_owner_token() {
    let adapter = Arc::new(WebAdapter::new(
        OWNER_TOKEN.to_owned(),
        OWNER_USER_ID.to_owned(),
    ));
    let app = axum::Router::new().nest("/chat", adapter.router());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local_addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });

    let session_key = SessionKey::new();
    let url = format!(
        "ws://{addr}/chat/ws?session_key={key}&token=wrong",
        key = session_key,
    );
    let result = tokio_tungstenite::connect_async(&url).await;
    assert!(result.is_err(), "wrong token must reject the upgrade");

    server.abort();
}
