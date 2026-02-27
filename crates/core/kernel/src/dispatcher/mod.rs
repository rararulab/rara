//! Agent dispatcher — priority work queue with session affinity.
//!
//! The dispatcher serializes tasks per session key while parallelizing across
//! different sessions.  Actual agent execution is delegated to a
//! [`TaskExecutor`] implementation provided by the consumer (typically the
//! workers crate).

pub mod core;
pub mod error;
pub mod executor;
pub mod log_store;
pub mod metrics;
pub mod types;

pub use self::core::AgentDispatcher;
pub use executor::TaskExecutor;
pub use log_store::{DispatcherLogStore, InMemoryLogStore};
pub use types::*;
