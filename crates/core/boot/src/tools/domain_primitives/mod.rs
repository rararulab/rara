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

//! Domain primitives: application-specific atomic operations (db, notify,
//! storage).

mod composio;
mod db_mutate;
mod db_query;
mod notify;
#[cfg(feature = "k8s")]
pub mod pod;
mod send_email;
mod storage_read;

pub use composio::ComposioTool;
pub use db_mutate::DbMutateTool;
pub use db_query::DbQueryTool;
pub use notify::NotifyTool;
#[cfg(feature = "k8s")]
pub use pod::PodTool;
pub use send_email::SendEmailTool;
pub use storage_read::StorageReadTool;
