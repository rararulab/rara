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

//! Background worker implementations for job automation.
//!
//! This crate contains concrete worker implementations that orchestrate
//! domain services for background processing tasks.

pub mod agent_scheduler;
pub mod jd_parser;
pub mod proactive;
pub mod saved_job_analyze;
pub mod saved_job_crawl;
pub mod saved_job_gc;
pub mod scheduled_agent;
pub mod system_routes;
pub mod tools;
pub mod types;
pub mod worker_state;
