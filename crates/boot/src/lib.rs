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

//! Kernel bootstrap — assembles a production-ready `Kernel` from external
//! dependencies.
//!
//! The primary entry point is [`kernel::boot()`], which creates a
//! fully-configured `Kernel` with its I/O subsystem (buses, stream hub,
//! endpoint registry, ingress pipeline).

pub mod agentfs;
pub mod audit;
pub mod bus;
pub mod components;
pub mod composio;
pub mod error;
pub mod guard;
pub mod kernel;
pub mod llm_registry;
pub mod manifests;
pub mod mcp;
pub mod outbox;
pub mod resolvers;
pub mod skills;
pub mod state;
pub mod stream;
pub mod tape_convert;
pub mod tools;
pub mod user_store;
