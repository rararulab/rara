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

//! End-to-end tests for the `WebAdapter` inbound code path.
//!
//! These tests are NOT kernel tests — they drive the web adapter directly
//! via its test-only inbound handler so the adapter itself (audio
//! transcription routing, `RawPlatformMessage` construction, session
//! resolution, kernel submission) is the system under test.
//!
//! The kernel is still booted via [`TestKernelBuilder`] so the adapter has a
//! real `KernelHandle` to submit to, and assertions read from real turn
//! traces.

use std::{path::PathBuf, sync::Once, time::Duration};

use rara_channels::web::WebAdapter;
use rara_kernel::{
    channel::{
        adapter::ChannelAdapter,
        types::{ContentBlock, MessageContent},
    },
    session::SessionKey,
    testing::{TestKernelBuilder, scripted_response},
};
use rara_stt::{SttConfig, SttService};
use serde_json::json;
use tokio::time::{Instant, sleep};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

/// CI runners can be noisy under full-workspace `nextest`; keep a generous
/// upper bound for end-to-end completion checks.
const TURN_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Override rara_paths directories to a writable temp path so tests
/// don't touch `~/.config/rara`.
fn init_test_env() {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    static INIT: Once = Once::new();
    let root = ROOT.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("rara-test-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create stable test env root");
        dir
    });
    INIT.call_once(move || {
        let data = root.join("rara_data");
        let config = root.join("rara_config");
        std::fs::create_dir_all(&data).expect("create stable test data dir");
        std::fs::create_dir_all(&config).expect("create stable test config dir");
        rara_paths::set_custom_data_dir(&data);
        rara_paths::set_custom_config_dir(&config);
    });
}

/// Poll `list_processes` until at least one session exists, returning its key.
async fn wait_for_first_session(handle: &rara_kernel::handle::KernelHandle) -> SessionKey {
    let deadline = Instant::now() + TURN_WAIT_TIMEOUT;
    loop {
        let processes = handle.list_processes();
        if let Some(first) = processes.first() {
            return first.session_key;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a session to be created by the adapter"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

/// Poll until the session has at least `expected_turns` completed turns.
async fn wait_for_turn_count(
    handle: &rara_kernel::handle::KernelHandle,
    session_key: SessionKey,
    expected_turns: usize,
) {
    let deadline = Instant::now() + TURN_WAIT_TIMEOUT;
    loop {
        let traces = handle.get_process_turns(session_key);
        if traces.len() >= expected_turns {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for turn {expected_turns} in session {session_key}; \
             current_turns={} latest_trace={:?}",
            traces.len(),
            traces.last()
        );
        sleep(Duration::from_millis(50)).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A text message handed to the web adapter must reach the kernel as a
/// resolved user message, spawn a session, and produce a turn whose reply
/// matches the scripted LLM response.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn web_text_message_reaches_kernel() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();

    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            scripted_response("hello back"),
            // Padding for any auxiliary LLM calls the kernel may issue.
            scripted_response("(padding)"),
        ])
        .build()
        .await;

    let adapter = WebAdapter::new(
        "test-owner-token".to_owned(),
        "test-user".to_owned(),
        rara_channels::web_reply_buffer::test_config(),
    );
    adapter
        .start(tk.handle.clone())
        .await
        .expect("adapter start");

    adapter
        .handle_inbound_for_test(
            "web-session-alpha",
            "test-user",
            MessageContent::Text("hello".to_owned()),
        )
        .await
        .expect("handle_inbound_for_test");

    let session_key = wait_for_first_session(&tk.handle).await;
    wait_for_turn_count(&tk.handle, session_key, 1).await;

    let traces = tk.handle.get_process_turns(session_key);
    assert_eq!(traces.len(), 1, "should have exactly 1 turn");
    let turn = &traces[0];
    assert!(turn.success, "turn should succeed: {:?}", turn.error);

    let preview = turn
        .iterations
        .last()
        .map(|i| i.text_preview.as_str())
        .unwrap_or("");
    assert!(
        preview.contains("hello back"),
        "expected scripted response in preview, got: {preview}"
    );

    tk.shutdown();
}

/// An inbound message carrying a base64 audio block must be routed through
/// the STT service (mocked by wiremock) before reaching the kernel. The
/// kernel should then see the transcribed text, not raw audio bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn web_audio_message_is_transcribed_via_stt() {
    let tmp = tempfile::tempdir().expect("tempdir");
    init_test_env();

    // Spin up a fake STT server that returns a known transcription.
    let mock_stt = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"text": "transcribed voice"})),
        )
        .mount(&mock_stt)
        .await;

    let stt_config = SttConfig::builder()
        .base_url(mock_stt.uri())
        .model("whisper-1".to_owned())
        .build();
    let stt = SttService::from_config(&stt_config);

    let tk = TestKernelBuilder::new(tmp.path())
        .responses(vec![
            scripted_response("got your voice"),
            scripted_response("(padding)"),
        ])
        .build()
        .await;

    let adapter = WebAdapter::new(
        "test-owner-token".to_owned(),
        "test-user".to_owned(),
        rara_channels::web_reply_buffer::test_config(),
    )
    .with_stt_service(Some(stt));
    adapter
        .start(tk.handle.clone())
        .await
        .expect("adapter start");

    // Build a multimodal payload with a single (fake) audio clip.
    use base64::Engine;
    let fake_audio = base64::engine::general_purpose::STANDARD.encode(b"fake-ogg-bytes");
    let content = MessageContent::Multimodal(vec![ContentBlock::AudioBase64 {
        media_type: "audio/webm".to_owned(),
        data:       fake_audio,
    }]);

    adapter
        .handle_inbound_for_test("web-session-voice", "test-user", content)
        .await
        .expect("handle_inbound_for_test");

    let session_key = wait_for_first_session(&tk.handle).await;
    wait_for_turn_count(&tk.handle, session_key, 1).await;

    // The kernel must have replied via the scripted driver — this only
    // happens if the adapter successfully transcribed the audio and
    // submitted a text message.
    let traces = tk.handle.get_process_turns(session_key);
    let preview = traces
        .last()
        .and_then(|t| t.iterations.last())
        .map(|i| i.text_preview.as_str())
        .unwrap_or("");
    assert!(
        preview.contains("got your voice"),
        "scripted reply missing — STT transcription likely did not run; got: {preview}"
    );

    // And the STT mock must have been hit at least once.
    let requests = mock_stt
        .received_requests()
        .await
        .expect("wiremock received_requests");
    assert!(
        !requests.is_empty(),
        "STT mock server received no requests — adapter bypassed transcription"
    );

    tk.shutdown();
}
