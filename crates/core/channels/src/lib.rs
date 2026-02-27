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

//! # rara-channels
//!
//! Channel adapter implementations for rara-kernel's [`ChannelAdapter`] trait.
//!
//! ## Web Adapter
//!
//! The [`web::WebAdapter`] provides real-time communication via:
//! - **WebSocket** (`GET /ws`) — bidirectional message streaming
//! - **SSE** (`GET /events`) — server-push event stream
//! - **POST** (`POST /messages`) — send a message (companion to SSE)
//!
//! The adapter does not start its own server; instead it exposes an
//! [`axum::Router`] that the application mounts into its HTTP server.

pub mod web;
