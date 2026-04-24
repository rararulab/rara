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

//! HTTP admin surface for the kernel scheduler.
//!
//! Exposes read-only curation of scheduled jobs under
//! `/api/v1/scheduler/*`. Creation is intentionally NOT exposed here —
//! jobs are only registered via the agent-facing `schedule` tool so the
//! audit trail stays tied to the originating session's principal.

pub mod dto;
pub mod router;
pub mod service;

pub use router::scheduler_routes;
pub use service::SchedulerSvc;
