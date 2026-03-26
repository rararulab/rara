//! Speech-to-Text (STT) service for transcribing audio to text.

mod config;
mod service;

pub use config::SttConfig;
pub use service::SttService;
