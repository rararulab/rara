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

//! OpenTelemetry metrics for the kernel.
//!
//! Organized by domain: process lifecycle, LLM turns, event processing, I/O
//! pipeline. All metrics use `LazyLock` for static registration via the
//! global OpenTelemetry meter.

use std::sync::LazyLock;

use opentelemetry::{
    KeyValue, global,
    metrics::{Counter, Histogram, UpDownCounter},
};

/// Return the shared meter scoped to the kernel crate.
fn meter() -> opentelemetry::metrics::Meter { global::meter("rara-kernel") }

// -- Session lifecycle -------------------------------------------------------

/// Total agent sessions created.
static SESSION_CREATED: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("kernel.session.created")
        .with_description("Total agent sessions created")
        .build()
});

/// Total agent sessions suspended (idle timeout / done).
static SESSION_SUSPENDED: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("kernel.session.suspended")
        .with_description("Total agent sessions suspended")
        .build()
});

/// Currently active agent sessions (gauge via up-down counter).
static SESSION_ACTIVE: LazyLock<UpDownCounter<i64>> = LazyLock::new(|| {
    meter()
        .i64_up_down_counter("kernel.session.active")
        .with_description("Currently active agent sessions")
        .build()
});

// -- LLM turn metrics --------------------------------------------------------

/// Total LLM turns executed.
static TURN_TOTAL: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("kernel.turn.total")
        .with_description("Total LLM turns executed")
        .build()
});

/// LLM turn execution duration in seconds.
static TURN_DURATION_SECONDS: LazyLock<Histogram<f64>> = LazyLock::new(|| {
    meter()
        .f64_histogram("kernel.turn.duration")
        .with_description("LLM turn execution duration in seconds")
        .with_unit("s")
        .build()
});

/// Total tool calls made during turns.
static TURN_TOOL_CALLS: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("kernel.turn.tool_calls")
        .with_description("Total tool calls made during turns")
        .build()
});

/// Total input tokens consumed.
static TURN_TOKENS_INPUT: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("kernel.turn.tokens.input")
        .with_description("Total input tokens consumed")
        .build()
});

/// Total output tokens produced.
static TURN_TOKENS_OUTPUT: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("kernel.turn.tokens.output")
        .with_description("Total output tokens produced")
        .build()
});

// -- Tool execution ----------------------------------------------------------

/// Per-tool execution duration in seconds.
static TOOL_DURATION: LazyLock<Histogram<f64>> = LazyLock::new(|| {
    meter()
        .f64_histogram("kernel.tool.duration")
        .with_description("Per-tool execution duration in seconds")
        .with_unit("s")
        .build()
});

// -- Event processing --------------------------------------------------------

/// Total events processed by the event loop.
static EVENT_PROCESSED: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("kernel.event.processed")
        .with_description("Total events processed")
        .build()
});

/// Total syscalls processed.
static SYSCALL_TOTAL: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("kernel.syscall.total")
        .with_description("Total syscalls processed")
        .build()
});

// -- I/O pipeline ------------------------------------------------------------

/// Total inbound messages received.
static MESSAGE_INBOUND: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("kernel.message.inbound")
        .with_description("Total inbound messages received")
        .build()
});

/// Total outbound messages delivered.
static MESSAGE_OUTBOUND: LazyLock<Counter<u64>> = LazyLock::new(|| {
    meter()
        .u64_counter("kernel.message.outbound")
        .with_description("Total outbound messages delivered")
        .build()
});

// -- Public helpers ----------------------------------------------------------

/// Record aggregate metrics for a completed LLM turn.
pub fn record_turn_metrics(
    agent_name: &str,
    model: &str,
    duration_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
) {
    let attrs = &[
        KeyValue::new("agent_name", agent_name.to_string()),
        KeyValue::new("model", model.to_string()),
    ];
    TURN_TOTAL.add(1, attrs);
    TURN_DURATION_SECONDS.record(duration_ms as f64 / 1_000.0, attrs);

    let model_attr = &[KeyValue::new("model", model.to_string())];
    TURN_TOKENS_INPUT.add(input_tokens, model_attr);
    TURN_TOKENS_OUTPUT.add(output_tokens, model_attr);
}

/// Record a tool call during an LLM turn.
pub fn record_turn_tool_call(agent_name: &str, tool_name: &str) {
    TURN_TOOL_CALLS.add(
        1,
        &[
            KeyValue::new("agent_name", agent_name.to_string()),
            KeyValue::new("tool_name", tool_name.to_string()),
        ],
    );
}

/// Record tool execution duration.
pub fn record_tool_duration(agent_name: &str, tool_name: &str, duration_ms: u64) {
    TOOL_DURATION.record(
        duration_ms as f64 / 1_000.0,
        &[
            KeyValue::new("agent_name", agent_name.to_string()),
            KeyValue::new("tool_name", tool_name.to_string()),
        ],
    );
}

/// Record a session creation event.
pub fn record_session_created(agent_name: &str) {
    SESSION_CREATED.add(1, &[KeyValue::new("agent_name", agent_name.to_string())]);
}

/// Record a session suspension event.
pub fn record_session_suspended(agent_name: &str, exit_state: &str) {
    SESSION_SUSPENDED.add(
        1,
        &[
            KeyValue::new("agent_name", agent_name.to_string()),
            KeyValue::new("exit_state", exit_state.to_string()),
        ],
    );
}

/// Increment the active session gauge.
pub fn inc_session_active(agent_name: &str) {
    SESSION_ACTIVE.add(1, &[KeyValue::new("agent_name", agent_name.to_string())]);
}

/// Decrement the active session gauge.
pub fn dec_session_active(agent_name: &str) {
    SESSION_ACTIVE.add(-1, &[KeyValue::new("agent_name", agent_name.to_string())]);
}

/// Record an event processed by the event loop.
pub fn record_event_processed(event_type: &str) {
    EVENT_PROCESSED.add(1, &[KeyValue::new("event_type", event_type.to_string())]);
}

/// Record a syscall processed.
pub fn record_syscall(syscall_type: &str) {
    SYSCALL_TOTAL.add(
        1,
        &[KeyValue::new("syscall_type", syscall_type.to_string())],
    );
}

/// Record an inbound message.
pub fn record_message_inbound(channel_type: &str) {
    MESSAGE_INBOUND.add(
        1,
        &[KeyValue::new("channel_type", channel_type.to_string())],
    );
}

/// Record an outbound message.
pub fn record_message_outbound(channel_type: &str) {
    MESSAGE_OUTBOUND.add(
        1,
        &[KeyValue::new("channel_type", channel_type.to_string())],
    );
}
