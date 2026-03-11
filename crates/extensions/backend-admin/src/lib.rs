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

//! # rara-backend-admin
//!
//! Unified HTTP admin routes for all backend subsystems: settings,
//! models, MCP servers, skills, and domain routes (chat).

pub mod agents;
pub mod auth;
pub mod chat;
pub mod kernel;
pub mod mcp;
pub mod settings;
pub mod skills;
pub mod state;
pub mod system_routes;
