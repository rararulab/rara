//! STT service implementation — see Task 2.

/// HTTP client for an OpenAI-compatible `/v1/audio/transcriptions` endpoint.
#[derive(Debug, Clone)]
pub struct SttService {
    _private: (),
}
