//! Telegram contacts allowlist.
//!
//! Provides persistent storage for approved Telegram contacts. Only contacts
//! in this table can receive outbound messages via the `send_telegram` tool.

pub mod error;
pub mod repository;
pub mod router;
pub mod types;
