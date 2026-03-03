// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Device abstraction — MCP servers and external APIs as hot-pluggable devices.
//!
//! A [`Device`] represents any external capability provider (MCP server,
//! external API, or internal service) that can be connected and disconnected
//! at runtime. Each device exposes a set of tool names (capabilities) and
//! reports its health status.
//!
//! # Architecture
//!
//! ```text
//! DeviceRegistry
//!   ├── DeviceId("mcp-github") → Arc<dyn Device>
//!   ├── DeviceId("mcp-slack")  → Arc<dyn Device>
//!   └── tool_to_device index
//!         ├── "github_create_pr" → DeviceId("mcp-github")
//!         └── "slack_send_msg"   → DeviceId("mcp-slack")
//! ```

pub mod registry;

use std::fmt;

use async_trait::async_trait;
use serde::Serialize;

use crate::error::Result;

// ---------------------------------------------------------------------------
// DeviceId
// ---------------------------------------------------------------------------

/// Unique identifier for a device.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct DeviceId(pub String);

impl DeviceId {
    /// Create a new `DeviceId` from any string-like value.
    pub fn new(id: impl Into<String>) -> Self { Self(id.into()) }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

impl<S: Into<String>> From<S> for DeviceId {
    fn from(s: S) -> Self { Self(s.into()) }
}

// ---------------------------------------------------------------------------
// DeviceType
// ---------------------------------------------------------------------------

/// The kind of device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, strum::Display)]
pub enum DeviceType {
    /// An MCP (Model Context Protocol) server.
    #[strum(serialize = "MCP Server")]
    McpServer,
    /// An external HTTP/gRPC API.
    #[strum(serialize = "External API")]
    ExternalApi,
    /// An internal service provided by the platform.
    #[strum(serialize = "Internal")]
    Internal,
}

// ---------------------------------------------------------------------------
// DeviceStatus
// ---------------------------------------------------------------------------

/// Runtime status of a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DeviceStatus {
    /// Device is connected and healthy.
    Connected,
    /// Device has been disconnected (either explicitly or after failure).
    Disconnected,
    /// Device encountered an error.
    Error(String),
    /// Device is starting up / connecting.
    Initializing,
}

impl fmt::Display for DeviceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected => f.write_str("Connected"),
            Self::Disconnected => f.write_str("Disconnected"),
            Self::Error(msg) => write!(f, "Error: {msg}"),
            Self::Initializing => f.write_str("Initializing"),
        }
    }
}

// ---------------------------------------------------------------------------
// DeviceInfo
// ---------------------------------------------------------------------------

/// Snapshot of a device's current state.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    /// Unique device identifier.
    pub id:           DeviceId,
    /// Human-readable device name.
    pub name:         String,
    /// What kind of device this is.
    pub device_type:  DeviceType,
    /// Current runtime status.
    pub status:       DeviceStatus,
    /// Tool names (capabilities) provided by this device.
    pub capabilities: Vec<String>,
    /// Arbitrary device-specific metadata.
    pub metadata:     serde_json::Value,
}

// ---------------------------------------------------------------------------
// Device trait
// ---------------------------------------------------------------------------

/// A hot-pluggable device that provides tools/capabilities to the kernel.
///
/// Implementors include MCP server wrappers, external API adapters, and
/// internal platform services. Devices can be registered and unregistered
/// at runtime via the
/// [`DeviceRegistry`](crate::device::registry::DeviceRegistry).
#[async_trait]
pub trait Device: Send + Sync {
    /// The unique identifier for this device.
    fn id(&self) -> &DeviceId;

    /// Return a snapshot of the device's current state.
    fn info(&self) -> DeviceInfo;

    /// Perform a health check and return the current status.
    ///
    /// Implementations should probe the underlying service (e.g., ping an
    /// MCP server) and return the observed status.
    async fn health_check(&self) -> Result<DeviceStatus>;

    /// Gracefully shut down the device, releasing any resources.
    async fn shutdown(&self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// DeviceEvent
// ---------------------------------------------------------------------------

/// Events emitted by the device subsystem.
///
/// These are published to the kernel's
/// [`NotificationBus`](crate::notification::NotificationBus) when devices
/// change state.
#[derive(Debug, Clone)]
pub enum DeviceEvent {
    /// A device was successfully connected.
    Connected(DeviceId),
    /// A device was disconnected (unregistered or shut down).
    Disconnected(DeviceId),
    /// A device reported an error (e.g., failed health check).
    Error {
        device_id: DeviceId,
        error:     String,
    },
    /// The tools provided by a device changed.
    ToolsChanged {
        device_id: DeviceId,
        added:     Vec<String>,
        removed:   Vec<String>,
    },
}

impl fmt::Display for DeviceEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected(id) => write!(f, "device connected: {id}"),
            Self::Disconnected(id) => write!(f, "device disconnected: {id}"),
            Self::Error { device_id, error } => {
                write!(f, "device error: {device_id}: {error}")
            }
            Self::ToolsChanged {
                device_id,
                added,
                removed,
            } => {
                write!(
                    f,
                    "device tools changed: {device_id} (+{} -{} tools)",
                    added.len(),
                    removed.len()
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_display() {
        let id = DeviceId::new("mcp-github");
        assert_eq!(id.to_string(), "mcp-github");
    }

    #[test]
    fn device_id_from_str() {
        let id: DeviceId = "mcp-slack".into();
        assert_eq!(id.0, "mcp-slack");
    }

    #[test]
    fn device_id_from_string() {
        let id: DeviceId = String::from("mcp-jira").into();
        assert_eq!(id.0, "mcp-jira");
    }

    #[test]
    fn device_id_equality() {
        let a = DeviceId::new("device-1");
        let b = DeviceId::new("device-1");
        let c = DeviceId::new("device-2");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn device_status_display() {
        assert_eq!(DeviceStatus::Connected.to_string(), "Connected");
        assert_eq!(DeviceStatus::Disconnected.to_string(), "Disconnected");
        assert_eq!(DeviceStatus::Initializing.to_string(), "Initializing");
        assert_eq!(
            DeviceStatus::Error("timeout".to_string()).to_string(),
            "Error: timeout"
        );
    }

    #[test]
    fn device_type_display() {
        assert_eq!(DeviceType::McpServer.to_string(), "MCP Server");
        assert_eq!(DeviceType::ExternalApi.to_string(), "External API");
        assert_eq!(DeviceType::Internal.to_string(), "Internal");
    }

    #[test]
    fn device_event_display() {
        let ev = DeviceEvent::Connected(DeviceId::new("d1"));
        assert_eq!(ev.to_string(), "device connected: d1");

        let ev = DeviceEvent::Disconnected(DeviceId::new("d2"));
        assert_eq!(ev.to_string(), "device disconnected: d2");

        let ev = DeviceEvent::Error {
            device_id: DeviceId::new("d3"),
            error:     "connection refused".to_string(),
        };
        assert_eq!(ev.to_string(), "device error: d3: connection refused");

        let ev = DeviceEvent::ToolsChanged {
            device_id: DeviceId::new("d4"),
            added:     vec!["tool_a".to_string(), "tool_b".to_string()],
            removed:   vec!["tool_c".to_string()],
        };
        assert_eq!(ev.to_string(), "device tools changed: d4 (+2 -1 tools)");
    }

    #[test]
    fn device_info_serialization() {
        let info = DeviceInfo {
            id:           DeviceId::new("mcp-github"),
            name:         "GitHub MCP".to_string(),
            device_type:  DeviceType::McpServer,
            status:       DeviceStatus::Connected,
            capabilities: vec!["create_pr".to_string(), "list_issues".to_string()],
            metadata:     serde_json::json!({"version": "1.0"}),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["id"], "mcp-github");
        assert_eq!(json["name"], "GitHub MCP");
        assert_eq!(json["capabilities"].as_array().unwrap().len(), 2);
    }
}
