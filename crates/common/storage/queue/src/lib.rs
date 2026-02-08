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

//! Persistent append-only queue with memory-mapped I/O.
//!
//! Features:
//! - Memory-mapped files via mmap-io for efficient disk access
//! - CRC64 checksums for data integrity
//! - Time-based directory organization (YYYY/MM/DD)
//! - Configurable file rolling strategies (size, time, count)
//! - Thread-safe appender for concurrent writes

mod crc;
mod index;
mod io_worker;
mod manifest;
mod manifest_writer;
mod queue;
mod recovery;
mod tailer;

pub mod appender;
pub mod builder;
pub mod config;
pub mod error;
pub mod file;
pub mod message;
pub mod path;

pub use appender::Appender;
pub use builder::QueueBuilder;
pub use config::{FlushMode, QueueConfig, RollStrategy};
pub use error::{Error, Result};
pub use file::{DataFile, ReadOnlyDataFile};
pub use message::Message;
pub use queue::Queue;
pub use tailer::Tailer;
