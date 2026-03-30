//! Speech-to-Text (STT) service for transcribing audio to text.

mod config;
mod process;
mod service;

pub use config::SttConfig;
pub use process::WhisperProcess;
pub use service::SttService;
