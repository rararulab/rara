use std::{
    collections::{BinaryHeap, HashMap, HashSet},
    sync::Arc,
};

use rara_kernel::agent_context::AgentContext;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tracing::{info, warn};

use super::{
    error::{ChannelClosedSnafu, DispatcherError},
    log_store::{DispatcherLogStore, LogFilter},
    metrics,
    types::{
        AgentTask, AgentTaskKind, DispatcherCommand, DispatcherStatus, PrioritizedTask,
        QueuedTaskInfo, RunningTaskInfo, RunningTaskInner, ScheduledJobCallback, SessionPersister,
        TaskRecord, TaskResult, TaskStatus,
    },
};
use crate::builtin::{proactive::ProactiveAgent, scheduled::ScheduledAgent};

/// Central dispatcher that serializes same-session tasks and parallelizes
/// across different sessions.
pub struct AgentDispatcher {
    tx:        mpsc::Sender<DispatcherCommand>,
    running:   Arc<RwLock<HashMap<String, RunningTaskInner>>>,
    queue:     Arc<Mutex<BinaryHeap<PrioritizedTask>>>,
    log_store: Arc<dyn DispatcherLogStore>,
}

impl AgentDispatcher {
    /// Create a new dispatcher and spawn its run loop.
    pub fn new(
        ctx: Arc<dyn AgentContext>,
        session_persister: Arc<dyn SessionPersister>,
        job_callback: Arc<dyn ScheduledJobCallback>,
        log_store: Arc<dyn DispatcherLogStore>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<DispatcherCommand>(100);
        let running = Arc::new(RwLock::new(HashMap::<String, RunningTaskInner>::new()));
        let queue = Arc::new(Mutex::new(BinaryHeap::<PrioritizedTask>::new()));

        let (finish_tx, finish_rx) = mpsc::channel::<String>(64);

        tokio::spawn(run_loop(
            rx,
            Arc::clone(&running),
            Arc::clone(&queue),
            Arc::clone(&log_store),
            ctx,
            session_persister,
            job_callback,
            finish_tx,
            finish_rx,
        ));

        Self {
            tx,
            running,
            queue,
            log_store,
        }
    }

    /// Submit a task and return a receiver for the result.
    pub async fn submit(
        &self,
        task: AgentTask,
    ) -> Result<oneshot::Receiver<TaskResult>, DispatcherError> {
        let (result_tx, result_rx) = oneshot::channel();
        self.tx
            .send(DispatcherCommand::Submit { task, result_tx })
            .await
            .map_err(|_| ChannelClosedSnafu.build())?;
        Ok(result_rx)
    }

    /// Cancel a running or queued task.
    pub async fn cancel(&self, task_id: &str) -> Result<(), DispatcherError> {
        self.tx
            .send(DispatcherCommand::Cancel {
                task_id: task_id.to_owned(),
            })
            .await
            .map_err(|_| ChannelClosedSnafu.build())?;
        Ok(())
    }

    /// Snapshot current dispatcher status.
    pub async fn status(&self) -> DispatcherStatus {
        let running = {
            let map = self.running.read().await;
            map.values().map(|r| r.info.clone()).collect()
        };
        let queued = {
            let heap = self.queue.lock().await;
            heap.iter()
                .map(|pt| QueuedTaskInfo {
                    id:          pt.task.id.clone(),
                    kind:        pt.task.kind.clone(),
                    session_key: pt.task.session_key.clone(),
                    priority:    pt.task.priority,
                    created_at:  pt.task.created_at,
                })
                .collect()
        };
        let stats = self.log_store.stats().await;
        DispatcherStatus {
            running,
            queued,
            stats,
        }
    }

    /// Query execution history.
    pub async fn history(&self, filter: LogFilter) -> Vec<TaskRecord> {
        self.log_store.query(filter).await
    }
}

// ---------------------------------------------------------------------------
// Run loop
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    mut rx: mpsc::Receiver<DispatcherCommand>,
    running: Arc<RwLock<HashMap<String, RunningTaskInner>>>,
    queue: Arc<Mutex<BinaryHeap<PrioritizedTask>>>,
    log_store: Arc<dyn DispatcherLogStore>,
    ctx: Arc<dyn AgentContext>,
    session_persister: Arc<dyn SessionPersister>,
    job_callback: Arc<dyn ScheduledJobCallback>,
    finish_tx: mpsc::Sender<String>,
    mut finish_rx: mpsc::Receiver<String>,
) {
    info!("dispatcher run loop started");

    loop {
        tokio::select! {
            Some(cmd) = rx.recv() => {
                match cmd {
                    DispatcherCommand::Submit { task, result_tx } => {
                        handle_submit(
                            task,
                            result_tx,
                            &running,
                            &queue,
                            &log_store,
                            &ctx,
                            &session_persister,
                            &job_callback,
                            &finish_tx,
                        ).await;
                    }
                    DispatcherCommand::Cancel { task_id } => {
                        handle_cancel(&task_id, &queue, &log_store).await;
                    }
                }
            }
            Some(task_id) = finish_rx.recv() => {
                // Task finished: remove from running set.
                {
                    let mut map = running.write().await;
                    if let Some(inner) = map.remove(&task_id) {
                        metrics::DISPATCHER_RUNNING_TASKS
                            .with_label_values(&[inner.info.kind.label()])
                            .dec();
                    }
                }
                // Try dispatching more tasks from the queue.
                try_dispatch(
                    &running,
                    &queue,
                    &log_store,
                    &ctx,
                    &session_persister,
                    &job_callback,
                    &finish_tx,
                ).await;
            }
            else => {
                info!("dispatcher run loop exiting: all senders dropped");
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_submit(
    task: AgentTask,
    result_tx: oneshot::Sender<TaskResult>,
    running: &Arc<RwLock<HashMap<String, RunningTaskInner>>>,
    queue: &Arc<Mutex<BinaryHeap<PrioritizedTask>>>,
    log_store: &Arc<dyn DispatcherLogStore>,
    ctx: &Arc<dyn AgentContext>,
    session_persister: &Arc<dyn SessionPersister>,
    job_callback: &Arc<dyn ScheduledJobCallback>,
    finish_tx: &mpsc::Sender<String>,
) {
    let kind_label = task.kind.label().to_owned();
    let priority_label = task.priority.label().to_owned();

    // Dedup check.
    if let Some(ref dedup_key) = task.dedup_key {
        let is_dup = {
            let running_map = running.read().await;
            let queue_heap = queue.lock().await;

            // Check running tasks: any task whose dedup_key matches.
            let running_dup = running_map.values().any(|_r| {
                // Running tasks don't carry dedup_key; use id-based matching
                // via a secondary index. For simplicity, check queue only.
                false
            });

            let queue_dup = queue_heap
                .iter()
                .any(|pt| pt.task.dedup_key.as_deref() == Some(dedup_key.as_str()));

            running_dup || queue_dup
        };

        if is_dup {
            info!(
                task_id = %task.id,
                dedup_key = %dedup_key,
                "task deduped"
            );
            metrics::DISPATCHER_TASKS_DEDUPED
                .with_label_values(&[&kind_label])
                .inc();

            let record = TaskRecord {
                id:           task.id.clone(),
                kind:         task.kind.clone(),
                session_key:  task.session_key.clone(),
                priority:     task.priority,
                status:       TaskStatus::Deduped,
                submitted_at: task.created_at,
                started_at:   None,
                finished_at:  Some(jiff::Timestamp::now()),
                duration_ms:  None,
                error:        None,
                iterations:   None,
                tool_calls:   None,
            };
            log_store.append(record).await;

            let _ = result_tx.send(TaskResult {
                task_id: task.id,
                status:  TaskStatus::Deduped,
                output:  None,
                error:   None,
            });
            return;
        }
    }

    // Record submission.
    metrics::DISPATCHER_TASKS_SUBMITTED
        .with_label_values(&[&kind_label, &priority_label])
        .inc();

    let record = TaskRecord {
        id:           task.id.clone(),
        kind:         task.kind.clone(),
        session_key:  task.session_key.clone(),
        priority:     task.priority,
        status:       TaskStatus::Queued,
        submitted_at: task.created_at,
        started_at:   None,
        finished_at:  None,
        duration_ms:  None,
        error:        None,
        iterations:   None,
        tool_calls:   None,
    };
    log_store.append(record).await;

    // Enqueue.
    {
        let mut heap = queue.lock().await;
        heap.push(PrioritizedTask { task, result_tx });
        metrics::DISPATCHER_QUEUE_SIZE
            .with_label_values(&[&priority_label])
            .inc();
    }

    // Attempt immediate dispatch.
    try_dispatch(
        running,
        queue,
        log_store,
        ctx,
        session_persister,
        job_callback,
        finish_tx,
    )
    .await;
}

async fn handle_cancel(
    task_id: &str,
    queue: &Arc<Mutex<BinaryHeap<PrioritizedTask>>>,
    log_store: &Arc<dyn DispatcherLogStore>,
) {
    let mut heap = queue.lock().await;
    let mut remaining = Vec::with_capacity(heap.len());
    let mut cancelled = false;

    while let Some(pt) = heap.pop() {
        if pt.task.id == task_id {
            let priority_label = pt.task.priority.label().to_owned();
            metrics::DISPATCHER_QUEUE_SIZE
                .with_label_values(&[&priority_label])
                .dec();

            let record = TaskRecord {
                id:           pt.task.id.clone(),
                kind:         pt.task.kind.clone(),
                session_key:  pt.task.session_key.clone(),
                priority:     pt.task.priority,
                status:       TaskStatus::Cancelled,
                submitted_at: pt.task.created_at,
                started_at:   None,
                finished_at:  Some(jiff::Timestamp::now()),
                duration_ms:  None,
                error:        None,
                iterations:   None,
                tool_calls:   None,
            };
            log_store.append(record).await;

            let _ = pt.result_tx.send(TaskResult {
                task_id: task_id.to_owned(),
                status:  TaskStatus::Cancelled,
                output:  None,
                error:   None,
            });
            cancelled = true;
            info!(task_id, "task cancelled from queue");
        } else {
            remaining.push(pt);
        }
    }

    for pt in remaining {
        heap.push(pt);
    }

    if !cancelled {
        warn!(
            task_id,
            "cancel requested but task not found in queue (may be running)"
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn try_dispatch(
    running: &Arc<RwLock<HashMap<String, RunningTaskInner>>>,
    queue: &Arc<Mutex<BinaryHeap<PrioritizedTask>>>,
    log_store: &Arc<dyn DispatcherLogStore>,
    ctx: &Arc<dyn AgentContext>,
    session_persister: &Arc<dyn SessionPersister>,
    job_callback: &Arc<dyn ScheduledJobCallback>,
    finish_tx: &mpsc::Sender<String>,
) {
    // Determine busy sessions.
    let busy_sessions: HashSet<String> = {
        let map = running.read().await;
        map.values().map(|r| r.info.session_key.clone()).collect()
    };

    // Pop dispatchable tasks (session not busy).
    let mut to_dispatch = Vec::new();
    let mut blocked = Vec::new();
    {
        let mut heap = queue.lock().await;
        while let Some(pt) = heap.pop() {
            if busy_sessions.contains(&pt.task.session_key)
                || to_dispatch
                    .iter()
                    .any(|d: &PrioritizedTask| d.task.session_key == pt.task.session_key)
            {
                blocked.push(pt);
            } else {
                to_dispatch.push(pt);
            }
        }
        // Put blocked tasks back.
        for pt in blocked {
            heap.push(pt);
        }
    }

    // Spawn each dispatchable task.
    for pt in to_dispatch {
        let task_id = pt.task.id.clone();
        let kind = pt.task.kind.clone();
        let session_key = pt.task.session_key.clone();
        let priority = pt.task.priority;
        let created_at = pt.task.created_at;
        let priority_label = priority.label().to_owned();
        let kind_label = kind.label().to_owned();

        let started_at = jiff::Timestamp::now();

        // Record queue wait time.
        let wait_secs = (started_at.as_second() - created_at.as_second()).unsigned_abs() as f64;
        metrics::DISPATCHER_QUEUE_WAIT
            .with_label_values(&[&kind_label])
            .observe(wait_secs);

        metrics::DISPATCHER_QUEUE_SIZE
            .with_label_values(&[&priority_label])
            .dec();
        metrics::DISPATCHER_RUNNING_TASKS
            .with_label_values(&[&kind_label])
            .inc();

        // Register as running.
        {
            let mut map = running.write().await;
            map.insert(
                task_id.clone(),
                RunningTaskInner {
                    info: RunningTaskInfo {
                        id: task_id.clone(),
                        kind: kind.clone(),
                        session_key: session_key.clone(),
                        priority,
                        started_at,
                    },
                },
            );
        }

        // Spawn execution.
        let ctx = Arc::clone(ctx);
        let session_persister = Arc::clone(session_persister);
        let job_callback = Arc::clone(job_callback);
        let log_store = Arc::clone(log_store);
        let finish_tx = finish_tx.clone();

        tokio::spawn(async move {
            let result =
                execute_task(&pt.task, &ctx, &*session_persister, &*job_callback).await;

            let finished_at = jiff::Timestamp::now();
            let duration_ms =
                (finished_at.as_millisecond() - started_at.as_millisecond()).unsigned_abs();
            let duration_secs = duration_ms as f64 / 1000.0;

            let (status, error, iterations, tool_calls) = match &result {
                Ok(output) => (
                    TaskStatus::Completed,
                    None,
                    Some(output.iterations),
                    Some(output.tool_calls_made),
                ),
                Err(e) => (TaskStatus::Error, Some(e.to_string()), None, None),
            };

            let status_label = if status == TaskStatus::Completed {
                "completed"
            } else {
                "error"
            };
            metrics::DISPATCHER_TASKS_COMPLETED
                .with_label_values(&[kind_label.as_str(), status_label])
                .inc();
            metrics::DISPATCHER_TASK_DURATION
                .with_label_values(&[&kind_label])
                .observe(duration_secs);

            let record = TaskRecord {
                id: task_id.clone(),
                kind: kind.clone(),
                session_key: session_key.clone(),
                priority,
                status: status.clone(),
                submitted_at: created_at,
                started_at: Some(started_at),
                finished_at: Some(finished_at),
                duration_ms: Some(duration_ms),
                error: error.clone(),
                iterations,
                tool_calls,
            };
            log_store.append(record).await;

            let task_result = match result {
                Ok(output) => TaskResult {
                    task_id: task_id.clone(),
                    status,
                    output: Some(output),
                    error: None,
                },
                Err(_) => TaskResult {
                    task_id: task_id.clone(),
                    status,
                    output: None,
                    error,
                },
            };
            let _ = pt.result_tx.send(task_result);

            // Signal the run loop that this task finished.
            let _ = finish_tx.send(task_id).await;
        });
    }
}

#[tracing::instrument(
    skip(task, ctx, session_persister, job_callback),
    fields(
        task_id = %task.id,
        task_kind = %task.kind,
        session = %task.session_key,
    )
)]
async fn execute_task(
    task: &AgentTask,
    ctx: &Arc<dyn AgentContext>,
    session_persister: &dyn SessionPersister,
    job_callback: &dyn ScheduledJobCallback,
) -> Result<crate::builtin::AgentOutput, crate::orchestrator::error::OrchestratorError> {
    // Ensure session exists.
    session_persister.ensure_session(&task.session_key).await;

    match &task.kind {
        AgentTaskKind::Proactive => {
            let agent = ProactiveAgent::new(Arc::clone(ctx));
            let output = agent.run(&task.message, &task.history).await?;

            // Persist conversation turns.
            let user_prompt = ProactiveAgent::build_user_prompt(&task.message);
            session_persister
                .persist_raw_message(
                    &task.session_key,
                    &rara_sessions::types::ChatMessage::user(&user_prompt),
                )
                .await
                .ok();
            session_persister
                .persist_raw_message(
                    &task.session_key,
                    &rara_sessions::types::ChatMessage::assistant(&output.response_text),
                )
                .await
                .ok();

            info!(
                iterations = output.iterations,
                tool_calls = output.tool_calls_made,
                "proactive task completed"
            );
            Ok(output)
        }
        AgentTaskKind::Scheduled { job_id } => {
            let agent = ScheduledAgent::new(Arc::clone(ctx));
            let history = if task.history.is_empty() {
                None
            } else {
                Some(task.history.as_slice())
            };
            let output = agent.run(&task.message, history).await?;

            // Persist conversation turns.
            if let Err(e) = session_persister
                .persist_messages(&task.session_key, &task.message, &output.response_text)
                .await
            {
                warn!(
                    job_id = %job_id,
                    error = %e,
                    "failed to persist scheduled agent session messages"
                );
            }

            // Mark job executed.
            if let Err(e) = job_callback.mark_executed(job_id).await {
                warn!(
                    job_id = %job_id,
                    error = %e,
                    "failed to mark scheduled job executed"
                );
            }

            info!(
                job_id = %job_id,
                iterations = output.iterations,
                tool_calls = output.tool_calls_made,
                "scheduled task completed"
            );
            Ok(output)
        }
        AgentTaskKind::Pipeline => {
            info!("pipeline task kind is not yet implemented");
            Ok(crate::builtin::AgentOutput {
                response_text:   "pipeline not implemented".to_owned(),
                iterations:      0,
                tool_calls_made: 0,
                truncated:       false,
            })
        }
    }
}
