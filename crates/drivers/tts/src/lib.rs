//! Text-to-Speech (TTS) HTTP client for OpenAI-compatible speech synthesis
//! endpoints.

mod config;
mod error;
mod service;

pub use config::TtsConfig;
pub use error::{Result, TtsError};
pub use service::{AudioOutput, TtsService};
