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

//! Domain model structs for the job automation platform.
//!
//! Each module contains the entity struct(s) and associated status enums
//! that map directly to the PostgreSQL schema.

pub mod ai;
pub mod job;
pub mod metrics;

pub use ai::{AiModelProvider, AiRun, PromptKind, PromptTemplate};
pub use job::{Job, JobStatus};
pub use metrics::{MetricsPeriod, MetricsSnapshot};
