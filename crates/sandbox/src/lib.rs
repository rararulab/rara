//! Hardware-isolated code execution sandbox for rara.
//!
//! `rara-sandbox` wraps the [`boxlite`] microVM runtime and exposes a small,
//! concrete API surface — `Sandbox`, `SandboxConfig`, `ExecRequest`, and
//! `ExecOutcome` — that the kernel's Tool subsystem can use to run untrusted
//! code with hardware-level isolation.
//!
//! # Design
//!
//! This crate intentionally exposes **concrete types** rather than a
//! `SandboxBackend` trait. We expect exactly one backend (boxlite) for the
//! foreseeable future; adding a trait now would be speculative abstraction
//! (closed as YAGNI in #1697). If a second backend ever appears, the trait
//! can be extracted without changing this crate's semantics.
//!
//! # Example
//!
//! ```no_run
//! use futures::StreamExt;
//! use rara_sandbox::{ExecRequest, Sandbox, SandboxConfig};
//!
//! # async fn demo() -> rara_sandbox::Result<()> {
//! let config = SandboxConfig::builder()
//!     .rootfs_image("alpine:latest".to_owned())
//!     .build();
//! let sandbox = Sandbox::create(config).await?;
//! let mut outcome = sandbox
//!     .exec(
//!         ExecRequest::builder()
//!             .command("echo")
//!             .args(vec!["hi".to_owned()])
//!             .build(),
//!     )
//!     .await?;
//! while let Some(line) = outcome.stdout.next().await {
//!     println!("{line}");
//! }
//! sandbox.destroy().await?;
//! # Ok(())
//! # }
//! ```
//!
//! See `AGENT.md` at the crate root for the boxlite integration footguns
//! (git-only dependency, runtime file staging, submodules) that new agents
//! working here need to know about.

mod config;
mod error;
mod sandbox;

pub use config::{ExecRequest, SandboxConfig};
pub use error::{BoxliteSnafu, Result, SandboxError};
pub use sandbox::{ExecOutcome, Sandbox};
