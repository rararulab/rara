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
//! pipeline. All metrics use `lazy_static` for static registration with the
//! global prometheus registry.

use lazy_static::lazy_static;
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

// -- Process lifecycle -------------------------------------------------------

lazy_static! {
    /// Total agent processes spawned.
    pub static ref PROCESS_SPAWNED: IntCounterVec =
        register_int_counter_vec!(
            "kernel_process_spawned_total",
            "Total agent processes spawned",
            &[AGENT_NAME_LABEL]
        ).unwrap();

    /// Total agent processes completed.
    pub static ref PROCESS_COMPLETED: IntCounterVec =
        register_int_counter_vec!(
            "kernel_process_completed_total",
            "Total agent processes completed",
            &[AGENT_NAME_LABEL, EXIT_STATE_LABEL]
        ).unwrap();

    /// Currently active agent processes.
    pub static ref PROCESS_ACTIVE: IntGaugeVec =
        register_int_gauge_vec!(
            "kernel_process_active",
            "Currently active agent processes",
            &[AGENT_NAME_LABEL]
        ).unwrap();
}

// -- LLM turn metrics --------------------------------------------------------

lazy_static! {
    /// Total LLM turns executed.
    pub static ref TURN_TOTAL: IntCounterVec =
        register_int_counter_vec!(
            "kernel_turn_total",
            "Total LLM turns executed",
            &[AGENT_NAME_LABEL, MODEL_LABEL]
        ).unwrap();

    /// LLM turn execution duration in seconds.
    pub static ref TURN_DURATION_SECONDS: HistogramVec =
        register_histogram_vec!(
            "kernel_turn_duration_seconds",
            "LLM turn execution duration in seconds",
            &[AGENT_NAME_LABEL, MODEL_LABEL],
            vec![0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0]
        ).unwrap();

    /// Total tool calls made during turns.
    pub static ref TURN_TOOL_CALLS: IntCounterVec =
        register_int_counter_vec!(
            "kernel_turn_tool_calls_total",
            "Total tool calls made during turns",
            &[AGENT_NAME_LABEL, TOOL_NAME_LABEL]
        ).unwrap();

    /// Total input tokens consumed.
    pub static ref TURN_TOKENS_INPUT: IntCounterVec =
        register_int_counter_vec!(
            "kernel_turn_tokens_input_total",
            "Total input tokens consumed",
            &[MODEL_LABEL]
        ).unwrap();

    /// Total output tokens produced.
    pub static ref TURN_TOKENS_OUTPUT: IntCounterVec =
        register_int_counter_vec!(
            "kernel_turn_tokens_output_total",
            "Total output tokens produced",
            &[MODEL_LABEL]
        ).unwrap();
}

// -- Event processing --------------------------------------------------------

lazy_static! {
    /// Total events processed by the event loop.
    pub static ref EVENT_PROCESSED: IntCounterVec =
        register_int_counter_vec!(
            "kernel_event_processed_total",
            "Total events processed",
            &[EVENT_TYPE_LABEL]
        ).unwrap();

    /// Total syscalls processed.
    pub static ref SYSCALL_TOTAL: IntCounterVec =
        register_int_counter_vec!(
            "kernel_syscall_total",
            "Total syscalls processed",
            &[SYSCALL_TYPE_LABEL]
        ).unwrap();
}

// -- I/O pipeline ------------------------------------------------------------

lazy_static! {
    /// Total inbound messages received.
    pub static ref MESSAGE_INBOUND: IntCounterVec =
        register_int_counter_vec!(
            "kernel_message_inbound_total",
            "Total inbound messages received",
            &[CHANNEL_TYPE_LABEL]
        ).unwrap();

    /// Total outbound messages delivered.
    pub static ref MESSAGE_OUTBOUND: IntCounterVec =
        register_int_counter_vec!(
            "kernel_message_outbound_total",
            "Total outbound messages delivered",
            &[CHANNEL_TYPE_LABEL]
        ).unwrap();
}

/// Force-initialize all metrics so they appear in `/metrics` output
/// immediately.
pub fn init() {
    lazy_static::initialize(&PROCESS_SPAWNED);
    lazy_static::initialize(&PROCESS_COMPLETED);
    lazy_static::initialize(&PROCESS_ACTIVE);
    lazy_static::initialize(&TURN_TOTAL);
    lazy_static::initialize(&TURN_DURATION_SECONDS);
    lazy_static::initialize(&TURN_TOOL_CALLS);
    lazy_static::initialize(&TURN_TOKENS_INPUT);
    lazy_static::initialize(&TURN_TOKENS_OUTPUT);
    lazy_static::initialize(&EVENT_PROCESSED);
    lazy_static::initialize(&SYSCALL_TOTAL);
    lazy_static::initialize(&MESSAGE_INBOUND);
    lazy_static::initialize(&MESSAGE_OUTBOUND);
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn metric_family(name: &str) -> prometheus::proto::MetricFamily {
        prometheus::gather()
            .into_iter()
            .find(|family| family.name() == name)
            .unwrap_or_else(|| panic!("missing metric family: {name}"))
    }

    #[test]
    fn test_init_does_not_panic() { init(); }

    #[test]
    fn test_metrics_gatherable_after_use() {
        // Metrics only appear in gather() after at least one sample is recorded.
        PROCESS_SPAWNED.with_label_values(&["test"]).inc();
        TURN_TOTAL.with_label_values(&["test", "gpt-4"]).inc();
        MESSAGE_INBOUND.with_label_values(&["web"]).inc();

        let families = prometheus::gather();
        let names: Vec<&str> = families.iter().map(|f| f.name()).collect();
        assert!(names.contains(&"kernel_process_spawned_total"));
        assert!(names.contains(&"kernel_turn_total"));
        assert!(names.contains(&"kernel_message_inbound_total"));
    }

    #[test]
    fn test_record_turn_metrics_emits_all_series() {
        let agent_name = format!("agent-{}", ulid::Ulid::new());
        let model = format!("model-{}", ulid::Ulid::new());
        let tool_name = format!("tool-{}", ulid::Ulid::new());

        record_turn_metrics(&agent_name, &model, 1_250, 11, 23);
        record_turn_tool_call(&agent_name, &tool_name);

        let turn_total = metric_family("kernel_turn_total");
        assert!(turn_total.get_metric().iter().any(|metric| {
            let has_agent_label = metric
                .get_label()
                .iter()
                .any(|label| label.name() == AGENT_NAME_LABEL && label.value() == agent_name);
            let has_model_label = metric
                .get_label()
                .iter()
                .any(|label| label.name() == MODEL_LABEL && label.value() == model);
            has_agent_label
                && has_model_label
                && metric
                    .get_counter()
                    .as_ref()
                    .map_or(0.0, prometheus::proto::Counter::value)
                    >= 1.0
        }));

        let turn_duration = metric_family("kernel_turn_duration_seconds");
        assert!(turn_duration.get_metric().iter().any(|metric| {
            metric
                .get_histogram()
                .as_ref()
                .map_or(0, prometheus::proto::Histogram::sample_count)
                >= 1
        }));

        let turn_tool_calls = metric_family("kernel_turn_tool_calls_total");
        assert!(turn_tool_calls.get_metric().iter().any(|metric| {
            let has_agent_label = metric
                .get_label()
                .iter()
                .any(|label| label.name() == AGENT_NAME_LABEL && label.value() == agent_name);
            let has_tool_label = metric
                .get_label()
                .iter()
                .any(|label| label.name() == TOOL_NAME_LABEL && label.value() == tool_name);
            has_agent_label
                && has_tool_label
                && metric
                    .get_counter()
                    .as_ref()
                    .map_or(0.0, prometheus::proto::Counter::value)
                    >= 1.0
        }));

        let turn_tokens_input = metric_family("kernel_turn_tokens_input_total");
        assert!(turn_tokens_input.get_metric().iter().any(|metric| {
            metric
                .get_counter()
                .as_ref()
                .map_or(0.0, prometheus::proto::Counter::value)
                >= 11.0
        }));

        let turn_tokens_output = metric_family("kernel_turn_tokens_output_total");
        assert!(turn_tokens_output.get_metric().iter().any(|metric| {
            metric
                .get_counter()
                .as_ref()
                .map_or(0.0, prometheus::proto::Counter::value)
                >= 23.0
        }));
    }
}
