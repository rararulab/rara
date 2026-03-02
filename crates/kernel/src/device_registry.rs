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

//! Device registry — manages hot-pluggable devices and their tool mappings.
//!
//! The [`DeviceRegistry`] is the central place for registering and
//! unregistering devices at runtime. It maintains a bidirectional index
//! between device IDs and the tool names they provide, enabling the kernel
//! to look up which device supplies a given tool.
//!
//! # Thread Safety
//!
//! All internal state is protected by [`DashMap`], making the registry safe
//! for concurrent access from multiple tasks without external locking.

use std::sync::Arc;

use dashmap::DashMap;
use tracing::{info, warn};

use crate::{
    device::{Device, DeviceEvent, DeviceId, DeviceInfo, DeviceStatus},
    error::{KernelError, Result},
};

// ---------------------------------------------------------------------------
// DeviceRegistry
// ---------------------------------------------------------------------------

/// Shared reference to the [`DeviceRegistry`].
pub type DeviceRegistryRef = Arc<DeviceRegistry>;

/// Registry of hot-pluggable devices and their tool mappings.
///
/// Provides thread-safe registration, unregistration, and lookup of devices.
/// Each device declares a set of tool names (capabilities), and the registry
/// maintains a reverse index from tool name to the owning device.
pub struct DeviceRegistry {
    /// Active devices keyed by their unique ID.
    devices:        DashMap<DeviceId, Arc<dyn Device>>,
    /// Reverse index: tool name → device ID.
    tool_to_device: DashMap<String, DeviceId>,
}

impl DeviceRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            devices:        DashMap::new(),
            tool_to_device: DashMap::new(),
        }
    }

    /// Register a device, indexing its capabilities.
    ///
    /// Returns a [`DeviceEvent::Connected`] on success, which the caller can
    /// publish to the event bus.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::DeviceAlreadyRegistered`] if a device with the
    /// same ID is already registered.
    pub fn register(&self, device: Arc<dyn Device>) -> Result<DeviceEvent> {
        let id = device.id().clone();
        let info = device.info();

        if self.devices.contains_key(&id) {
            return Err(KernelError::DeviceAlreadyRegistered { id: id.0 });
        }

        // Index tool → device.
        for tool_name in &info.capabilities {
            self.tool_to_device.insert(tool_name.clone(), id.clone());
        }

        self.devices.insert(id.clone(), device);
        info!(device_id = %id, tools = ?info.capabilities, "device registered");

        Ok(DeviceEvent::Connected(id))
    }

    /// Unregister a device, removing its tool index entries.
    ///
    /// Returns a [`DeviceEvent::Disconnected`] on success.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::DeviceNotFound`] if no device with the given ID
    /// exists.
    pub fn unregister(&self, id: &DeviceId) -> Result<DeviceEvent> {
        let (_id, device) = self
            .devices
            .remove(id)
            .ok_or_else(|| KernelError::DeviceNotFound { id: id.0.clone() })?;

        // Remove tool → device mappings for this device.
        let info = device.info();
        for tool_name in &info.capabilities {
            self.tool_to_device.remove(tool_name);
        }

        info!(device_id = %id, "device unregistered");

        Ok(DeviceEvent::Disconnected(id.clone()))
    }

    /// Look up a device by its ID.
    pub fn get(&self, id: &DeviceId) -> Option<Arc<dyn Device>> {
        self.devices.get(id).map(|entry| Arc::clone(entry.value()))
    }

    /// List info snapshots of all registered devices.
    pub fn list(&self) -> Vec<DeviceInfo> {
        self.devices
            .iter()
            .map(|entry| entry.value().info())
            .collect()
    }

    /// Find which device provides a given tool.
    pub fn find_by_tool(&self, tool_name: &str) -> Option<DeviceId> {
        self.tool_to_device
            .get(tool_name)
            .map(|entry| entry.value().clone())
    }

    /// Return the number of registered devices.
    #[must_use]
    pub fn len(&self) -> usize { self.devices.len() }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.devices.is_empty() }

    /// Run health checks on all registered devices concurrently.
    ///
    /// Returns a vec of `(DeviceId, DeviceStatus)` pairs. Devices that fail
    /// health checks will have [`DeviceStatus::Error`] in the result.
    pub async fn health_check_all(&self) -> Vec<(DeviceId, DeviceStatus)> {
        let devices: Vec<(DeviceId, Arc<dyn Device>)> = self
            .devices
            .iter()
            .map(|entry| (entry.key().clone(), Arc::clone(entry.value())))
            .collect();

        let mut results = Vec::with_capacity(devices.len());
        for (id, device) in devices {
            let status = match device.health_check().await {
                Ok(status) => status,
                Err(e) => {
                    warn!(device_id = %id, error = %e, "device health check failed");
                    DeviceStatus::Error(e.to_string())
                }
            };
            results.push((id, status));
        }
        results
    }

    /// Return all tool names mapped to their providing device IDs.
    pub fn tool_device_map(&self) -> Vec<(String, DeviceId)> {
        self.tool_to_device
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }
}

impl Default for DeviceRegistry {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::device::{DeviceStatus, DeviceType};

    /// A simple in-memory device for testing.
    struct FakeDevice {
        id:           DeviceId,
        name:         String,
        device_type:  DeviceType,
        capabilities: Vec<String>,
        healthy:      bool,
    }

    impl FakeDevice {
        fn new(id: &str, name: &str, tools: Vec<&str>, healthy: bool) -> Self {
            Self {
                id: DeviceId::new(id),
                name: name.to_string(),
                device_type: DeviceType::McpServer,
                capabilities: tools.into_iter().map(String::from).collect(),
                healthy,
            }
        }
    }

    #[async_trait]
    impl Device for FakeDevice {
        fn id(&self) -> &DeviceId { &self.id }

        fn info(&self) -> DeviceInfo {
            DeviceInfo {
                id:           self.id.clone(),
                name:         self.name.clone(),
                device_type:  self.device_type.clone(),
                status:       if self.healthy {
                    DeviceStatus::Connected
                } else {
                    DeviceStatus::Error("unhealthy".to_string())
                },
                capabilities: self.capabilities.clone(),
                metadata:     json!({}),
            }
        }

        async fn health_check(&self) -> Result<DeviceStatus> {
            if self.healthy {
                Ok(DeviceStatus::Connected)
            } else {
                Ok(DeviceStatus::Error("unhealthy".to_string()))
            }
        }

        async fn shutdown(&self) -> Result<()> { Ok(()) }
    }

    #[test]
    fn register_device() {
        let registry = DeviceRegistry::new();
        let device = Arc::new(FakeDevice::new(
            "mcp-github",
            "GitHub MCP",
            vec!["create_pr", "list_issues"],
            true,
        ));

        let event = registry.register(device).unwrap();
        assert!(matches!(event, DeviceEvent::Connected(ref id) if id.0 == "mcp-github"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn register_duplicate_device_fails() {
        let registry = DeviceRegistry::new();
        let d1 = Arc::new(FakeDevice::new("d1", "Device 1", vec!["tool_a"], true));
        let d1_dup = Arc::new(FakeDevice::new("d1", "Device 1 dup", vec!["tool_b"], true));

        registry.register(d1).unwrap();
        let err = registry.register(d1_dup).unwrap_err();
        assert!(err.to_string().contains("already registered"));
    }

    #[test]
    fn unregister_device() {
        let registry = DeviceRegistry::new();
        let device = Arc::new(FakeDevice::new(
            "mcp-slack",
            "Slack MCP",
            vec!["send_message", "list_channels"],
            true,
        ));

        registry.register(device).unwrap();
        assert_eq!(registry.len(), 1);

        let event = registry.unregister(&DeviceId::new("mcp-slack")).unwrap();
        assert!(matches!(event, DeviceEvent::Disconnected(ref id) if id.0 == "mcp-slack"));
        assert!(registry.is_empty());
    }

    #[test]
    fn unregister_nonexistent_device_fails() {
        let registry = DeviceRegistry::new();
        let err = registry
            .unregister(&DeviceId::new("nonexistent"))
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn get_device() {
        let registry = DeviceRegistry::new();
        let device = Arc::new(FakeDevice::new("d1", "Device 1", vec!["tool_a"], true));
        registry.register(device).unwrap();

        let found = registry.get(&DeviceId::new("d1"));
        assert!(found.is_some());
        assert_eq!(found.unwrap().id(), &DeviceId::new("d1"));

        let not_found = registry.get(&DeviceId::new("nonexistent"));
        assert!(not_found.is_none());
    }

    #[test]
    fn find_by_tool() {
        let registry = DeviceRegistry::new();
        let device = Arc::new(FakeDevice::new(
            "mcp-github",
            "GitHub MCP",
            vec!["create_pr", "list_issues"],
            true,
        ));
        registry.register(device).unwrap();

        let owner = registry.find_by_tool("create_pr");
        assert_eq!(owner, Some(DeviceId::new("mcp-github")));

        let owner = registry.find_by_tool("list_issues");
        assert_eq!(owner, Some(DeviceId::new("mcp-github")));

        let owner = registry.find_by_tool("unknown_tool");
        assert!(owner.is_none());
    }

    #[test]
    fn tool_index_cleaned_on_unregister() {
        let registry = DeviceRegistry::new();
        let device = Arc::new(FakeDevice::new(
            "d1",
            "Device",
            vec!["tool_a", "tool_b"],
            true,
        ));
        registry.register(device).unwrap();
        assert!(registry.find_by_tool("tool_a").is_some());
        assert!(registry.find_by_tool("tool_b").is_some());

        registry.unregister(&DeviceId::new("d1")).unwrap();
        assert!(registry.find_by_tool("tool_a").is_none());
        assert!(registry.find_by_tool("tool_b").is_none());
    }

    #[test]
    fn list_devices() {
        let registry = DeviceRegistry::new();
        let d1 = Arc::new(FakeDevice::new("d1", "Device 1", vec!["t1"], true));
        let d2 = Arc::new(FakeDevice::new("d2", "Device 2", vec!["t2"], true));
        registry.register(d1).unwrap();
        registry.register(d2).unwrap();

        let infos = registry.list();
        assert_eq!(infos.len(), 2);

        let names: Vec<&str> = infos.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"Device 1"));
        assert!(names.contains(&"Device 2"));
    }

    #[test]
    fn tool_device_map_returns_all_mappings() {
        let registry = DeviceRegistry::new();
        let device = Arc::new(FakeDevice::new(
            "d1",
            "Device 1",
            vec!["tool_a", "tool_b"],
            true,
        ));
        registry.register(device).unwrap();

        let map = registry.tool_device_map();
        assert_eq!(map.len(), 2);
        let tool_names: Vec<&str> = map.iter().map(|(name, _)| name.as_str()).collect();
        assert!(tool_names.contains(&"tool_a"));
        assert!(tool_names.contains(&"tool_b"));
    }

    #[tokio::test]
    async fn health_check_all_healthy() {
        let registry = DeviceRegistry::new();
        let d1 = Arc::new(FakeDevice::new("d1", "Healthy", vec!["t1"], true));
        let d2 = Arc::new(FakeDevice::new("d2", "Also Healthy", vec!["t2"], true));
        registry.register(d1).unwrap();
        registry.register(d2).unwrap();

        let results = registry.health_check_all().await;
        assert_eq!(results.len(), 2);
        for (_, status) in &results {
            assert_eq!(status, &DeviceStatus::Connected);
        }
    }

    #[tokio::test]
    async fn health_check_all_mixed() {
        let registry = DeviceRegistry::new();
        let healthy = Arc::new(FakeDevice::new("d1", "Healthy", vec!["t1"], true));
        let unhealthy = Arc::new(FakeDevice::new("d2", "Unhealthy", vec!["t2"], false));
        registry.register(healthy).unwrap();
        registry.register(unhealthy).unwrap();

        let results = registry.health_check_all().await;
        assert_eq!(results.len(), 2);

        let d1_status = results.iter().find(|(id, _)| id.0 == "d1").unwrap();
        assert_eq!(d1_status.1, DeviceStatus::Connected);

        let d2_status = results.iter().find(|(id, _)| id.0 == "d2").unwrap();
        assert!(matches!(d2_status.1, DeviceStatus::Error(_)));
    }

    #[tokio::test]
    async fn health_check_empty_registry() {
        let registry = DeviceRegistry::new();
        let results = registry.health_check_all().await;
        assert!(results.is_empty());
    }

    #[test]
    fn multiple_devices_with_distinct_tools() {
        let registry = DeviceRegistry::new();
        let d1 = Arc::new(FakeDevice::new(
            "github",
            "GitHub",
            vec!["create_pr", "merge_pr"],
            true,
        ));
        let d2 = Arc::new(FakeDevice::new(
            "slack",
            "Slack",
            vec!["send_msg", "list_channels"],
            true,
        ));
        registry.register(d1).unwrap();
        registry.register(d2).unwrap();

        assert_eq!(
            registry.find_by_tool("create_pr"),
            Some(DeviceId::new("github"))
        );
        assert_eq!(
            registry.find_by_tool("send_msg"),
            Some(DeviceId::new("slack"))
        );
    }

    #[test]
    fn register_unregister_reregister() {
        let registry = DeviceRegistry::new();
        let device = Arc::new(FakeDevice::new("d1", "Device", vec!["tool_a"], true));

        registry.register(device).unwrap();
        assert_eq!(registry.len(), 1);

        registry.unregister(&DeviceId::new("d1")).unwrap();
        assert!(registry.is_empty());

        // Re-register with different tools.
        let device2 = Arc::new(FakeDevice::new("d1", "Device v2", vec!["tool_b"], true));
        registry.register(device2).unwrap();
        assert_eq!(registry.len(), 1);

        assert!(registry.find_by_tool("tool_a").is_none());
        assert_eq!(registry.find_by_tool("tool_b"), Some(DeviceId::new("d1")));
    }
}
