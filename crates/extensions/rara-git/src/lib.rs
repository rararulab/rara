pub mod error;
pub mod ssh;

pub use error::GitError;
pub use ssh::{get_or_create_keypair, get_public_key, SshKeyPair};
