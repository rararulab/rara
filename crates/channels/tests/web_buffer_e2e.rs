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

//! End-to-end coverage for the always-on per-session reply buffer wired into
//! `WebAdapter` (issue #1804).
//!
//! Two scenarios:
//!
//! - **Happy-path egress routing** — covers the egress-sink contract for an
//!   already-routed Web `Reply`: build a `Web` endpoint with a real
//!   `connection_id`, call `WebAdapter::send` with a `PlatformOutbound::Reply`,
//!   and assert a live subscriber sees a matching `WebEvent::Message`.
//! - **Listener-loss recovery** — same `WebAdapter::send` invocation, but the
//!   subscriber is dropped before publish. A fresh subscriber then drains the
//!   reply buffer and observes the missed event.
//!
//! These tests do not boot a kernel; they drive `ChannelAdapter::send`
//! directly because the buffering bug lives in the adapter sink, not in
//! origin-endpoint resolution.

use std::{sync::Arc, time::Duration};

use rara_channels::{
    web::{WebAdapter, WebEvent},
    web_reply_buffer::ReplyBuffer,
};
use rara_kernel::{
    channel::{adapter::ChannelAdapter, types::ChannelType},
    io::{Endpoint, EndpointAddress, PlatformOutbound},
    session::SessionKey,
};

fn web_endpoint(session_key: &SessionKey) -> Endpoint {
    Endpoint {
        channel_type: ChannelType::Web,
        address:      EndpointAddress::Web {
            connection_id: session_key.to_string(),
        },
    }
}

/// Simulates a connected WS / SSE session by lazily creating the
/// per-session broadcast bus and returning a fresh subscription on
/// it — mirroring what the real WS handler does at connect time.
fn subscribe(
    adapter: &WebAdapter,
    session_key: &SessionKey,
) -> tokio::sync::broadcast::Receiver<WebEvent> {
    adapter.subscribe_for_test(session_key)
}

#[tokio::test]
async fn happy_path_reply_reaches_subscribed_listener() {
    let buffer = ReplyBuffer::new(rara_channels::web_reply_buffer::test_config());
    let adapter = WebAdapter::new(
        "tok".to_owned(),
        "user".to_owned(),
        rara_channels::web_reply_buffer::test_config(),
    )
    .with_reply_buffer(Arc::clone(&buffer));

    let session_key = SessionKey::new();
    let mut rx = subscribe(&adapter, &session_key);

    let endpoint = web_endpoint(&session_key);
    adapter
        .send(
            &endpoint,
            PlatformOutbound::Reply {
                content:       "task complete".to_owned(),
                attachments:   Vec::new(),
                reply_context: None,
            },
        )
        .await
        .expect("egress send");

    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("listener received event before timeout")
        .expect("broadcast not closed");

    match event {
        WebEvent::Message { content } => assert_eq!(content, "task complete"),
        other => panic!("expected WebEvent::Message, got {other:?}"),
    }

    // The buffer must also have captured the reply so a hypothetical
    // reconnecting tab could replay it.
    let buffered = buffer.snapshot(&session_key);
    assert_eq!(buffered.len(), 1, "buffer should retain the reply");
}

#[tokio::test]
async fn listener_loss_is_recovered_via_buffer_snapshot() {
    let buffer = ReplyBuffer::new(rara_channels::web_reply_buffer::test_config());
    let adapter = WebAdapter::new(
        "tok".to_owned(),
        "user".to_owned(),
        rara_channels::web_reply_buffer::test_config(),
    )
    .with_reply_buffer(Arc::clone(&buffer));

    let session_key = SessionKey::new();

    // First "tab" subscribes, then closes before the reply arrives.
    {
        let _rx = subscribe(&adapter, &session_key);
        // _rx is dropped at end of scope, dropping the only listener.
    }

    let endpoint = web_endpoint(&session_key);
    adapter
        .send(
            &endpoint,
            PlatformOutbound::Reply {
                content:       "while-you-were-away".to_owned(),
                attachments:   Vec::new(),
                reply_context: None,
            },
        )
        .await
        .expect("egress send while no listener");

    // The broadcast had zero receivers at publish time, so a
    // pre-#1804 build would have lost the reply forever. With the
    // buffer, a reconnecting tab drains the snapshot.
    let backlog = buffer.snapshot(&session_key);
    assert_eq!(backlog.len(), 1, "exactly one buffered event");
    match &backlog[0] {
        WebEvent::Message { content } => assert_eq!(content, "while-you-were-away"),
        other => panic!("expected WebEvent::Message, got {other:?}"),
    }
}

/// Regression for the original #1882 P0: the WS / SSE reattach path must
/// drain the buffer atomically with subscription, so a *second* reattach
/// inside the TTL window does not replay events the first reattach
/// already received. Pre-fix, both reattach sites called `subscribe()` +
/// `snapshot()` (snapshot does not clear), so the second reattach
/// re-played the same events — visible as duplicate messages on tab
/// refresh.
#[tokio::test]
async fn second_reattach_does_not_replay_drained_events() {
    let buffer = ReplyBuffer::new(rara_channels::web_reply_buffer::test_config());
    let adapter = WebAdapter::new(
        "tok".to_owned(),
        "user".to_owned(),
        rara_channels::web_reply_buffer::test_config(),
    )
    .with_reply_buffer(Arc::clone(&buffer));

    let session_key = SessionKey::new();
    let endpoint = web_endpoint(&session_key);

    // Publish with no listener attached — lands in the buffer.
    adapter
        .send(
            &endpoint,
            PlatformOutbound::Reply {
                content:       "missed-while-away".to_owned(),
                attachments:   Vec::new(),
                reply_context: None,
            },
        )
        .await
        .expect("egress send while no listener");

    // First reattach: drains the backlog.
    let (_rx1, backlog1) = adapter.reattach_for_test(&session_key);
    assert_eq!(backlog1.len(), 1, "first reattach drains the backlog");
    match &backlog1[0] {
        WebEvent::Message { content } => assert_eq!(content, "missed-while-away"),
        other => panic!("expected WebEvent::Message, got {other:?}"),
    }
    // Drop the first "WS" by dropping its receiver.
    drop(_rx1);

    // Second reattach to the same session_key inside the TTL window must
    // see ZERO buffered events — the first drain cleared the buffer.
    let (_rx2, backlog2) = adapter.reattach_for_test(&session_key);
    assert!(
        backlog2.is_empty(),
        "second reattach must not replay drained events; got {} events",
        backlog2.len()
    );
}
