//! Prometheus metrics for the kernel.
//!
//! All metrics use `LazyLock` for zero-cost static registration with the
//! global prometheus registry.

use std::sync::LazyLock;

use prometheus::{
    HistogramVec, IntCounterVec, IntGaugeVec, register_histogram_vec, register_int_counter_vec,
    register_int_gauge_vec,
};

// -- Process lifecycle -------------------------------------------------------

pub static PROCESS_SPAWNED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_process_spawned_total",
        "Total agent processes spawned",
        &["agent_name"]
    )
    .unwrap()
});

pub static PROCESS_COMPLETED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_process_completed_total",
        "Total agent processes completed",
        &["agent_name", "exit_state"]
    )
    .unwrap()
});

pub static PROCESS_ACTIVE: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "kernel_process_active",
        "Currently active agent processes",
        &["agent_name"]
    )
    .unwrap()
});

// -- LLM turn metrics --------------------------------------------------------

pub static TURN_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_total",
        "Total LLM turns executed",
        &["agent_name", "model"]
    )
    .unwrap()
});

pub static TURN_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "kernel_turn_duration_seconds",
        "LLM turn execution duration in seconds",
        &["agent_name", "model"],
        vec![0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0]
    )
    .unwrap()
});

pub static TURN_TOOL_CALLS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_tool_calls_total",
        "Total tool calls made during turns",
        &["agent_name", "tool_name"]
    )
    .unwrap()
});

pub static TURN_TOKENS_INPUT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_tokens_input_total",
        "Total input tokens consumed",
        &["model"]
    )
    .unwrap()
});

pub static TURN_TOKENS_OUTPUT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_tokens_output_total",
        "Total output tokens produced",
        &["model"]
    )
    .unwrap()
});

// -- Event processing --------------------------------------------------------

pub static EVENT_PROCESSED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_event_processed_total",
        "Total events processed",
        &["event_type"]
    )
    .unwrap()
});

pub static SYSCALL_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_syscall_total",
        "Total syscalls processed",
        &["syscall_type"]
    )
    .unwrap()
});

// -- I/O pipeline ------------------------------------------------------------

pub static MESSAGE_INBOUND: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_message_inbound_total",
        "Total inbound messages received",
        &["channel_type"]
    )
    .unwrap()
});

pub static MESSAGE_OUTBOUND: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_message_outbound_total",
        "Total outbound messages delivered",
        &["channel_type"]
    )
    .unwrap()
});
