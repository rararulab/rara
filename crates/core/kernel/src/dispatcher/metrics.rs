use std::sync::LazyLock;

use prometheus::{
    HistogramVec, IntCounterVec, IntGaugeVec, register_histogram_vec, register_int_counter_vec,
    register_int_gauge_vec,
};

pub static DISPATCHER_TASKS_SUBMITTED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "dispatcher_tasks_submitted_total",
        "Total tasks submitted to the dispatcher",
        &["kind", "priority"]
    )
    .unwrap()
});

pub static DISPATCHER_TASKS_COMPLETED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "dispatcher_tasks_completed_total",
        "Total tasks completed by the dispatcher",
        &["kind", "status"]
    )
    .unwrap()
});

pub static DISPATCHER_TASKS_DEDUPED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "dispatcher_tasks_deduped_total",
        "Total tasks deduped by the dispatcher",
        &["kind"]
    )
    .unwrap()
});

pub static DISPATCHER_QUEUE_SIZE: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "dispatcher_queue_size",
        "Current number of tasks in the dispatcher queue",
        &["priority"]
    )
    .unwrap()
});

pub static DISPATCHER_RUNNING_TASKS: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "dispatcher_running_tasks",
        "Current number of running tasks",
        &["kind"]
    )
    .unwrap()
});

pub static DISPATCHER_TASK_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "dispatcher_task_duration_seconds",
        "Duration of task execution in seconds",
        &["kind"]
    )
    .unwrap()
});

pub static DISPATCHER_QUEUE_WAIT: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "dispatcher_queue_wait_seconds",
        "Time tasks spend waiting in the queue",
        &["kind"]
    )
    .unwrap()
});
