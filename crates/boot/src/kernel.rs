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

//! Kernel boot APIs moved to `rara-kernel`.
//!
//! Build the kernel directly:
//!
//! ```rust,ignore
//! let mut io = rara_kernel::io::IOSubsystem::new(ingress_pipeline);
//! io.register_adapter(channel_type, adapter);
//!
//! let kernel = rara_kernel::kernel::Kernel::new(
//!     Default::default(),
//!     driver_registry,
//!     tool_registry,
//!     agent_registry,
//!     session_index,
//!     tape_service,
//!     settings,
//!     security,
//!     io,
//! );
//! ```
