//! HTTP integration tests for `TtsService` using a real in-process
//! wiremock server.
//!
//! These exercise the real HTTP/JSON code path — request serialization,
//! bearer-auth header, response byte decoding, and error mapping — without
//! mocking `TtsService` itself.

use rara_tts::{TtsConfig, TtsError, TtsService};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

async fn make_server() -> MockServer { MockServer::start().await }

fn config_for(server: &MockServer) -> TtsConfig {
    // `TtsService` appends `/audio/speech` to `base_url`, so include `/v1`
    // in the base URL to hit the OpenAI-style path.
    TtsConfig::builder()
        .base_url(format!("{}/v1", server.uri()))
        .api_key("test-key".to_owned())
        .model("tts-1".to_owned())
        .voice("alloy".to_owned())
        .format("opus".to_owned())
        .build()
}

#[tokio::test]
async fn happy_path_returns_audio_bytes() {
    let server = make_server().await;
    let fake_audio: Vec<u8> = vec![0x4F, 0x67, 0x67, 0x53]; // "OggS" magic bytes
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(fake_audio.clone()))
        .mount(&server)
        .await;

    let svc = TtsService::from_config(&config_for(&server));
    let result = svc.synthesize("hello world").await.expect("synthesize ok");
    assert_eq!(result.data, fake_audio);
    assert_eq!(result.mime_type, "audio/ogg;codecs=opus");
}

#[tokio::test]
async fn authorization_header_is_sent() {
    // Request is only matched when the bearer token is present; absent the
    // header wiremock returns 404 and the test fails.
    let server = make_server().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 4]))
        .mount(&server)
        .await;

    let svc = TtsService::from_config(&config_for(&server));
    svc.synthesize("hello")
        .await
        .expect("synthesize must succeed when auth header matches");
}

#[tokio::test]
async fn server_error_is_mapped_to_server_variant() {
    let server = make_server().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
        .mount(&server)
        .await;

    let svc = TtsService::from_config(&config_for(&server));
    let result = svc.synthesize("hello").await;
    assert!(
        matches!(result, Err(TtsError::Server { status: 500, .. })),
        "expected Server(500), got {result:?}"
    );
}

#[tokio::test]
async fn text_too_long_is_rejected_client_side() {
    // Server is started but should NOT be hit — expect(0) enforces this.
    let server = make_server().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let config = TtsConfig::builder()
        .base_url(format!("{}/v1", server.uri()))
        .model("tts-1".to_owned())
        .voice("alloy".to_owned())
        .format("opus".to_owned())
        .max_text_length(10_usize)
        .build();

    let svc = TtsService::from_config(&config);
    let result = svc
        .synthesize("this text is definitely longer than 10 chars")
        .await;
    assert!(
        matches!(result, Err(TtsError::TextTooLong { max: 10, .. })),
        "expected TextTooLong, got {result:?}"
    );
}
