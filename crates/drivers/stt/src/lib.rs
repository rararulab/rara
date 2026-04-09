//! Speech-to-Text (STT) service for transcribing audio to text.

mod config;
mod error;
mod process;
mod service;

pub use config::{SttConfig, SttCorrectionConfig};
pub use error::{Result, SttError};
pub use process::WhisperProcess;
pub use service::SttService;
