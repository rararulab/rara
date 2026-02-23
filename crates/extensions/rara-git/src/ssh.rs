use std::path::{Path, PathBuf};

use crate::error::GitError;

/// An SSH Ed25519 key pair.
#[derive(Debug, Clone)]
pub struct SshKeyPair {
    /// OpenSSH-formatted public key string (e.g. "ssh-ed25519 AAAA...")
    pub public_key: String,
    /// Path to private key file
    pub private_key_path: PathBuf,
}

/// Get or create an Ed25519 SSH key pair in the given directory.
///
/// Keys are stored as `{ssh_dir}/id_ed25519` and `{ssh_dir}/id_ed25519.pub`.
/// If keys already exist, they are loaded and returned. Otherwise, a new pair
/// is generated.
pub fn get_or_create_keypair(ssh_dir: &Path) -> Result<SshKeyPair, GitError> {
    let private_path = ssh_dir.join("id_ed25519");
    let public_path = ssh_dir.join("id_ed25519.pub");

    if private_path.exists() && public_path.exists() {
        let public_key = std::fs::read_to_string(&public_path).map_err(|e| {
            GitError::SshKey {
                message: format!("failed to read public key: {e}"),
            }
        })?;
        return Ok(SshKeyPair {
            public_key: public_key.trim().to_owned(),
            private_key_path: private_path,
        });
    }

    // Generate new Ed25519 key pair
    std::fs::create_dir_all(ssh_dir).map_err(|e| GitError::SshKey {
        message: format!("failed to create SSH directory: {e}"),
    })?;

    let private_key =
        ssh_key::PrivateKey::random(&mut rand_core::OsRng, ssh_key::Algorithm::Ed25519).map_err(
            |e| GitError::SshKey {
                message: format!("failed to generate key: {e}"),
            },
        )?;

    // Write private key (OpenSSH format, no passphrase)
    let private_pem = private_key
        .to_openssh(ssh_key::LineEnding::LF)
        .map_err(|e| GitError::SshKey {
            message: format!("failed to serialize private key: {e}"),
        })?;
    std::fs::write(&private_path, private_pem.as_bytes()).map_err(|e| GitError::SshKey {
        message: format!("failed to write private key: {e}"),
    })?;

    // Set permissions to 600 on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&private_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| GitError::SshKey {
                message: format!("failed to set key permissions: {e}"),
            })?;
    }

    // Write public key
    let public_key = private_key
        .public_key()
        .to_openssh()
        .map_err(|e| GitError::SshKey {
            message: format!("failed to serialize public key: {e}"),
        })?;
    std::fs::write(&public_path, public_key.as_bytes()).map_err(|e| GitError::SshKey {
        message: format!("failed to write public key: {e}"),
    })?;

    Ok(SshKeyPair {
        public_key: public_key.trim().to_owned(),
        private_key_path: private_path,
    })
}

/// Get the public key string, generating a key pair if none exists.
pub fn get_public_key(ssh_dir: &Path) -> Result<String, GitError> {
    let pair = get_or_create_keypair(ssh_dir)?;
    Ok(pair.public_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn generates_keypair_when_none_exists() {
        let tmp = TempDir::new().unwrap();
        let pair = get_or_create_keypair(tmp.path()).unwrap();
        assert!(!pair.public_key.is_empty());
        assert!(pair.public_key.starts_with("ssh-ed25519 "));
        assert!(tmp.path().join("id_ed25519").exists());
        assert!(tmp.path().join("id_ed25519.pub").exists());
    }

    #[test]
    fn returns_existing_keypair() {
        let tmp = TempDir::new().unwrap();
        let pair1 = get_or_create_keypair(tmp.path()).unwrap();
        let pair2 = get_or_create_keypair(tmp.path()).unwrap();
        assert_eq!(pair1.public_key, pair2.public_key);
    }

    #[test]
    fn get_public_key_creates_if_needed() {
        let tmp = TempDir::new().unwrap();
        let pk = get_public_key(tmp.path()).unwrap();
        assert!(pk.starts_with("ssh-ed25519 "));
    }
}
