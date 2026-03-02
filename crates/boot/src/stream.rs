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

//! Factory functions for StreamHub.

use std::sync::Arc;

use rara_kernel::io::stream::StreamHub;

/// Create a default StreamHub with the given per-stream broadcast capacity.
pub fn default_stream_hub(capacity: usize) -> Arc<StreamHub> { Arc::new(StreamHub::new(capacity)) }
