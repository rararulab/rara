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

//! Prometheus metrics for the kernel.
//!
//! Organized by domain: process lifecycle, LLM turns, event processing, I/O
//! pipeline. All metrics use `LazyLock` for static registration with the
//! global prometheus registry.

use std::sync::LazyLock;

use prometheus::*;

/// Agent name label.
pub const AGENT_NAME_LABEL: &str = "agent_name";
/// Model label.
pub const MODEL_LABEL: &str = "model";
/// Tool name label.
pub const TOOL_NAME_LABEL: &str = "tool_name";
/// Event type label.
pub const EVENT_TYPE_LABEL: &str = "event_type";
/// Syscall type label.
pub const SYSCALL_TYPE_LABEL: &str = "syscall_type";
/// Channel type label.
pub const CHANNEL_TYPE_LABEL: &str = "channel_type";
/// Exit state label.
pub const EXIT_STATE_LABEL: &str = "exit_state";

// -- Session lifecycle -------------------------------------------------------

/// Total agent sessions created.
pub static SESSION_CREATED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_session_created_total",
        "Total agent sessions created",
        &[AGENT_NAME_LABEL]
    )
    .unwrap()
});

/// Total agent sessions suspended (idle timeout / done).
pub static SESSION_SUSPENDED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_session_suspended_total",
        "Total agent sessions suspended",
        &[AGENT_NAME_LABEL, EXIT_STATE_LABEL]
    )
    .unwrap()
});

/// Currently active agent sessions.
pub static SESSION_ACTIVE: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "kernel_session_active",
        "Currently active agent sessions",
        &[AGENT_NAME_LABEL]
    )
    .unwrap()
});

// -- LLM turn metrics --------------------------------------------------------

/// Total LLM turns executed.
pub static TURN_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_total",
        "Total LLM turns executed",
        &[AGENT_NAME_LABEL, MODEL_LABEL]
    )
    .unwrap()
});

/// LLM turn execution duration in seconds.
pub static TURN_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "kernel_turn_duration_seconds",
        "LLM turn execution duration in seconds",
        &[AGENT_NAME_LABEL, MODEL_LABEL],
        vec![0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0]
    )
    .unwrap()
});

/// Total tool calls made during turns.
pub static TURN_TOOL_CALLS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_tool_calls_total",
        "Total tool calls made during turns",
        &[AGENT_NAME_LABEL, TOOL_NAME_LABEL]
    )
    .unwrap()
});

/// Total input tokens consumed.
pub static TURN_TOKENS_INPUT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_tokens_input_total",
        "Total input tokens consumed",
        &[MODEL_LABEL]
    )
    .unwrap()
});

/// Total output tokens produced.
pub static TURN_TOKENS_OUTPUT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_tokens_output_total",
        "Total output tokens produced",
        &[MODEL_LABEL]
    )
    .unwrap()
});

// -- Event processing --------------------------------------------------------

/// Total events processed by the event loop.
pub static EVENT_PROCESSED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_event_processed_total",
        "Total events processed",
        &[EVENT_TYPE_LABEL]
    )
    .unwrap()
});

/// Total syscalls processed.
pub static SYSCALL_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_syscall_total",
        "Total syscalls processed",
        &[SYSCALL_TYPE_LABEL]
    )
    .unwrap()
});

// -- I/O pipeline ------------------------------------------------------------

/// Total inbound messages received.
pub static MESSAGE_INBOUND: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_message_inbound_total",
        "Total inbound messages received",
        &[CHANNEL_TYPE_LABEL]
    )
    .unwrap()
});

/// Total outbound messages delivered.
pub static MESSAGE_OUTBOUND: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_message_outbound_total",
        "Total outbound messages delivered",
        &[CHANNEL_TYPE_LABEL]
    )
    .unwrap()
});

/// Record aggregate metrics for a completed LLM turn.
pub fn record_turn_metrics(
    agent_name: &str,
    model: &str,
    duration_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
) {
    TURN_TOTAL.with_label_values(&[agent_name, model]).inc();
    TURN_DURATION_SECONDS
        .with_label_values(&[agent_name, model])
        .observe(duration_ms as f64 / 1_000.0);
    TURN_TOKENS_INPUT
        .with_label_values(&[model])
        .inc_by(input_tokens);
    TURN_TOKENS_OUTPUT
        .with_label_values(&[model])
        .inc_by(output_tokens);
}

/// Record a tool call during an LLM turn.
pub fn record_turn_tool_call(agent_name: &str, tool_name: &str) {
    TURN_TOOL_CALLS
        .with_label_values(&[agent_name, tool_name])
        .inc();
}

// -- Tool execution ----------------------------------------------------------

/// Per-tool execution duration histogram.
pub static TOOL_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "kernel_tool_duration_seconds",
        "Tool execution duration in seconds",
        &[AGENT_NAME_LABEL, TOOL_NAME_LABEL],
        vec![0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0]
    )
    .unwrap()
});

/// Record tool execution duration for Prometheus.
pub fn record_tool_duration(agent_name: &str, tool_name: &str, duration_ms: u64) {
    TOOL_DURATION_SECONDS
        .with_label_values(&[agent_name, tool_name])
        .observe(duration_ms as f64 / 1000.0);
}
