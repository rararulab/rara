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

//! LLM turn execution and completion handling.

use std::sync::Arc;

use tracing::{error, info, info_span, warn};

use super::runtime::RuntimeTable;
use crate::{
    channel::types::ChatMessage,
    event::KernelEventEnvelope,
    io::{
        stream::StreamId,
        types::{InboundMessage, MessageId, OutboundEnvelope},
    },
    kernel::Kernel,
    process::{AgentRunLoopResult, SessionState},
    queue::EventQueueRef,
    session::{SessionIndex, SessionKey},
};

impl Kernel {}
