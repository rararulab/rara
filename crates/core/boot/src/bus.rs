// Copyright 2025 Crrow
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

//! Factory functions for I/O bus components.

use std::sync::Arc;

use rara_kernel::io::bus::{InboundBus, OutboundBus};
use rara_kernel::io::memory_bus::{InMemoryInboundBus, InMemoryOutboundBus};

/// Create a default in-memory inbound bus with the given capacity.
pub fn default_inbound_bus(capacity: usize) -> Arc<dyn InboundBus> {
    Arc::new(InMemoryInboundBus::new(capacity))
}

/// Create a default in-memory outbound bus with the given capacity.
pub fn default_outbound_bus(capacity: usize) -> Arc<dyn OutboundBus> {
    Arc::new(InMemoryOutboundBus::new(capacity))
}
