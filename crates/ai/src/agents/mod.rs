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

//! Task-specific AI agents.
//!
//! Each agent is a lightweight struct that borrows the underlying
//! OpenAI client and model name from [`AiService`](crate::service::AiService).

pub mod cover_letter;
pub mod follow_up;
pub mod interview_prep;
pub mod jd_analyzer;
pub mod jd_parser;
pub mod job_fit;
pub mod resume_optimizer;
