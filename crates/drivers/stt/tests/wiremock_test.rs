//! HTTP integration tests for `SttService` using a real in-process
//! wiremock server.
//!
//! These exercise the real HTTP/JSON code path end-to-end — multipart
//! serialization, response parsing, status-code handling, and retry
//! logic — without mocking `SttService` itself.

use rara_stt::{SttConfig, SttError, SttService};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn make_server() -> MockServer { MockServer::start().await }

fn config_for(server: &MockServer) -> SttConfig {
    SttConfig::builder()
        .base_url(server.uri())
        .model("whisper-1".to_owned())
        .build()
}

#[tokio::test]
async fn happy_path_returns_text() {
    let server = make_server().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "text": "hello world" })),
        )
        .mount(&server)
        .await;

    let svc = SttService::from_config(&config_for(&server));
    let result = svc.transcribe(b"fake audio".to_vec(), "audio/ogg").await;
    assert_eq!(result.expect("transcription should succeed"), "hello world");
}

#[tokio::test]
async fn empty_response_returns_empty_response_error() {
    let server = make_server().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "text": "" })))
        .mount(&server)
        .await;

    let svc = SttService::from_config(&config_for(&server));
    let result = svc.transcribe(b"fake audio".to_vec(), "audio/ogg").await;
    assert!(
        matches!(result, Err(SttError::EmptyResponse)),
        "expected EmptyResponse, got {result:?}"
    );
}

#[tokio::test]
async fn client_error_returns_server_error_without_retry() {
    // 4xx is not transient — service should surface it immediately.
    let server = make_server().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .expect(1) // must NOT be retried
        .mount(&server)
        .await;

    let svc = SttService::from_config(&config_for(&server));
    let result = svc.transcribe(b"fake audio".to_vec(), "audio/ogg").await;
    assert!(
        matches!(result, Err(SttError::ServerError { status: 400, .. })),
        "expected ServerError(400), got {result:?}"
    );
}

#[tokio::test]
async fn server_error_eventually_fails() {
    let server = make_server().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let svc = SttService::from_config(&config_for(&server));
    let result = svc.transcribe(b"fake audio".to_vec(), "audio/ogg").await;
    assert!(
        matches!(result, Err(SttError::ServerError { status: 500, .. })),
        "expected ServerError(500), got {result:?}"
    );
}

#[tokio::test]
async fn transient_error_then_success_retries() {
    // First request: 503, follow-up: 200. Retry logic should recover.
    let server = make_server().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "text": "after retry" })),
        )
        .mount(&server)
        .await;

    let svc = SttService::from_config(&config_for(&server));
    let result = svc.transcribe(b"fake audio".to_vec(), "audio/ogg").await;
    assert_eq!(result.expect("retry should succeed"), "after retry");
}

#[tokio::test]
async fn malformed_json_returns_parse_error() {
    let server = make_server().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("not valid json")
                .insert_header("content-type", "application/json"),
        )
        .mount(&server)
        .await;

    let svc = SttService::from_config(&config_for(&server));
    let result = svc.transcribe(b"fake audio".to_vec(), "audio/ogg").await;
    assert!(
        matches!(result, Err(SttError::Parse { .. })),
        "expected Parse error, got {result:?}"
    );
}
