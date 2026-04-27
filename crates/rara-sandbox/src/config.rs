//! Configuration types for sandbox creation and command execution.

use std::{path::PathBuf, time::Duration};

use bon::Builder;
use serde::{Deserialize, Serialize};

/// Configuration describing how a [`Sandbox`](crate::Sandbox) should be
/// provisioned.
///
/// Field values are passed through to boxlite without interpretation — in
/// particular the rootfs image reference MUST already be resolvable by the
/// host's boxlite image store. `rara-sandbox` never supplies a default image
/// in Rust: the application layer is responsible for reading the image name
/// from YAML config and passing it in here.
///
/// The struct derives [`Deserialize`] so application-layer code may load it
/// straight from YAML, e.g.:
///
/// ```yaml
/// sandbox:
///   rootfs_image: "alpine:latest"
///   name: "my-agent-sandbox"
///   volumes:
///     - host_path: "/Users/me/work"
///       guest_path: "/work"
///       read_only: false
///   network:
///     mode: enabled
///     allow_net: ["github.com"]
///   working_dir: "/work"
/// ```
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// OCI image reference passed to boxlite as
    /// [`RootfsSpec::Image`](boxlite::RootfsSpec::Image).
    pub rootfs_image: String,

    /// Optional human-readable box name. When `None`, boxlite generates one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Host-to-guest filesystem mounts forwarded to
    /// [`boxlite::BoxOptions::volumes`]. Empty means no extra mounts beyond
    /// what the rootfs image provides.
    #[serde(default)]
    #[builder(default)]
    pub volumes: Vec<VolumeMount>,

    /// Network policy forwarded to [`boxlite::BoxOptions::network`].
    ///
    /// Defaults to [`NetworkPolicy::Enabled`] with an empty allow-list, which
    /// matches boxlite's own default and preserves the historical `run_code`
    /// behavior of full outbound access.
    #[serde(default)]
    #[builder(default = NetworkPolicy::Enabled { allow_net: Vec::new() })]
    pub network: NetworkPolicy,

    /// Default working directory for commands executed in the sandbox.
    /// Forwarded to [`boxlite::BoxOptions::working_dir`]. `None` means the
    /// guest image's default (typically `/`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

/// Description of a single command to run inside a [`Sandbox`](crate::Sandbox).
///
/// Mirrors the subset of [`boxlite::BoxCommand`] that rara actually uses
/// today. Extra boxlite knobs (`tty`, `user`) can be added here when a
/// concrete caller needs them — not before, to keep the API surface minimal.
#[derive(Debug, Clone, Builder)]
pub struct ExecRequest {
    /// Executable to invoke inside the sandbox (e.g. `"echo"`, `"python"`).
    pub command: String,

    /// Arguments to pass, in order. Empty vec means no args.
    #[builder(default)]
    pub args: Vec<String>,

    /// Environment variables exported to the command.
    #[builder(default)]
    pub env: Vec<(String, String)>,

    /// Optional hard timeout enforced by boxlite.
    pub timeout: Option<Duration>,

    /// Per-exec working directory override. When `None`, boxlite falls back
    /// to [`SandboxConfig::working_dir`] (and ultimately the image default).
    pub working_dir: Option<String>,
}

/// A single host-to-guest filesystem mount.
///
/// Mirrors `boxlite::runtime::options::VolumeSpec` but keeps `host_path` typed
/// as [`PathBuf`] so callers compose it with the rest of the rara codebase
/// (which uses `PathBuf` everywhere) without stringly-typed paperwork. The
/// conversion to boxlite's `String`-typed `host_path` happens once in
/// [`Sandbox::create`](crate::Sandbox::create).
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct VolumeMount {
    /// Absolute path on the host. Boxlite requires absolute paths.
    pub host_path: PathBuf,

    /// Mount target inside the guest, e.g. `/work`.
    pub guest_path: String,

    /// When true, the mount is read-only from inside the guest.
    #[serde(default)]
    #[builder(default)]
    pub read_only: bool,
}

/// Network policy applied to the whole sandbox.
///
/// Wrapper over [`boxlite::NetworkSpec`]. We expose our own type because
/// `crates/rara-sandbox/AGENT.md` forbids re-exporting boxlite types — this
/// crate's job is to insulate the kernel from boxlite API churn.
///
/// # YAML form
///
/// Serialized with an internal `mode` tag so YAML stays human-readable and
/// the disabled variant doesn't need a dummy `allow_net` key:
///
/// ```yaml
/// network:
///   mode: enabled
///   allow_net: ["github.com", "*.crates.io"]
/// ```
///
/// ```yaml
/// network:
///   mode: disabled
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NetworkPolicy {
    /// Network is up. Empty `allow_net` means full outbound access; a
    /// non-empty list pins the guest to those hosts/CIDRs only.
    Enabled {
        /// Host patterns or CIDRs the guest may reach. See
        /// [`boxlite::NetworkSpec`] for the exact pattern grammar.
        #[serde(default)]
        allow_net: Vec<String>,
    },
    /// No network interface in the guest at all.
    Disabled,
}

impl Default for NetworkPolicy {
    /// Mirror boxlite's own [`NetworkSpec::default`](boxlite::NetworkSpec) —
    /// network up, empty allow-list = full outbound. Required so
    /// `#[serde(default)]` on [`SandboxConfig::network`] works when YAML
    /// omits the field; preserves `run_code`'s historical behavior.
    fn default() -> Self {
        Self::Enabled {
            allow_net: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_config_defaults_match_boxlite() {
        let cfg = SandboxConfig::builder()
            .rootfs_image("alpine:latest".to_owned())
            .build();

        assert!(cfg.volumes.is_empty());
        assert!(cfg.working_dir.is_none());
        match cfg.network {
            NetworkPolicy::Enabled { allow_net } => assert!(allow_net.is_empty()),
            NetworkPolicy::Disabled => panic!("default network must be Enabled"),
        }
    }

    #[test]
    fn network_policy_enabled_yaml_roundtrip() {
        let yaml = "mode: enabled\nallow_net:\n  - github.com\n";
        let parsed: NetworkPolicy = serde_yaml::from_str(yaml).unwrap();
        let allow = match &parsed {
            NetworkPolicy::Enabled { allow_net } => allow_net.clone(),
            NetworkPolicy::Disabled => panic!("expected Enabled"),
        };
        assert_eq!(allow, vec!["github.com".to_owned()]);

        let reserialized = serde_yaml::to_string(&parsed).unwrap();
        let reparsed: NetworkPolicy = serde_yaml::from_str(&reserialized).unwrap();
        assert!(matches!(reparsed, NetworkPolicy::Enabled { .. }));
    }

    #[test]
    fn network_policy_disabled_yaml_roundtrip() {
        let yaml = "mode: disabled\n";
        let parsed: NetworkPolicy = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(parsed, NetworkPolicy::Disabled));

        let reserialized = serde_yaml::to_string(&parsed).unwrap();
        let reparsed: NetworkPolicy = serde_yaml::from_str(&reserialized).unwrap();
        assert!(matches!(reparsed, NetworkPolicy::Disabled));
    }

    #[test]
    fn network_policy_enabled_default_allow_net() {
        // `mode: enabled` with no `allow_net` key should deserialize cleanly
        // — boxlite's own NetworkSpec uses the same shape.
        let parsed: NetworkPolicy = serde_yaml::from_str("mode: enabled\n").unwrap();
        match parsed {
            NetworkPolicy::Enabled { allow_net } => assert!(allow_net.is_empty()),
            NetworkPolicy::Disabled => panic!("expected Enabled"),
        }
    }

    #[test]
    fn volume_mount_builder() {
        let mount = VolumeMount::builder()
            .host_path(PathBuf::from("/tmp/host"))
            .guest_path("/work".to_owned())
            .read_only(true)
            .build();
        assert_eq!(mount.host_path, PathBuf::from("/tmp/host"));
        assert_eq!(mount.guest_path, "/work");
        assert!(mount.read_only);

        // read_only defaults to false when omitted.
        let rw = VolumeMount::builder()
            .host_path(PathBuf::from("/tmp/host"))
            .guest_path("/work".to_owned())
            .build();
        assert!(!rw.read_only);
    }
}
