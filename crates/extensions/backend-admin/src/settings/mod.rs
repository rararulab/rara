mod ai;
mod auth;
mod gmail;
pub mod ollama;
mod router;
pub mod service;
mod tg;

pub use router::routes;
pub use service::SettingsSvc;
