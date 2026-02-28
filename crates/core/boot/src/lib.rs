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

//! Kernel bootstrap — initializes trait-object components for `Kernel::new()`.

pub mod bus;
pub mod components;
pub mod error;
pub mod manifests;
pub mod mcp;
pub mod session;
pub mod skills;
pub mod stream;
pub mod tools;
pub mod user_store;
