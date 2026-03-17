# Scheduled & Background Tasks

Rara's kernel provides two mechanisms for agents to perform work outside the
main conversational loop: **scheduled tasks** (time-triggered) and **background
tasks** (immediately spawned). Both produce results that flow back to interested
sessions via a **tag-based notification bus**.

## Overview

```text
┌────────────┐   schedule-once/interval/cron   ┌───────────┐
│  Agent A   │ ──────────────────────────────►  │  JobWheel │
│ (session)  │                                  │ (on disk) │
└────────────┘                                  └─────┬─────┘
      │                                               │
      │  subscribe(tags=["pr_review"])                 │ timer fires
      ▼                                               ▼
┌──────────────────────┐                   ┌────────────────────┐
│ SubscriptionRegistry │                   │  Kernel spawns     │
│ (file-backed, by tag)│                   │  scheduled_job     │
└──────────────────────┘                   │  agent session     │
                                           └─────────┬──────────┘
                                                     │
                                                     │ kernel tool:
                                                     │ publish_report
                                                     ▼
                                           ┌────────────────────┐
                                           │ handle_publish_    │
                                           │ task_report()      │
                                           │ 1. persist result  │
                                           │ 2. match tags      │
                                           │ 3. deliver         │
                                           └─────────┬──────────┘
                                                     │
                              ┌───────────────────────┼──────────────────┐
                              ▼                       ▼                  ▼
                     ProactiveTurn           SilentAppend          (no match)
                  (synthetic message →     (append to sub's
                   triggers LLM turn)       tape silently)
```

## Scheduled Tasks

### Creating a Scheduled Job

Agents create scheduled jobs by calling one of three tools:

| Tool | Trigger | Example |
|------|---------|---------|
| `schedule-once` | Fire once after N seconds | Review PR in 5 minutes |
| `schedule-interval` | Fire every N seconds | Check deploy status every 60s |
| `schedule-cron` | 6-field cron expression | Daily standup report at 9am |

All three accept an optional `tags` array for notification routing:

```json
{
  "action": "schedule-once",
  "after_seconds": 300,
  "message": "Review PR #42 on rararulab/rara",
  "tags": ["pr_review", "repo:rararulab/rara"]
}
```

### How Tags Flow

1. The agent includes `tags` when creating the job
2. Tags are stored in `JobEntry` and persisted to disk by `JobWheel`
3. When the job fires, tags are injected into the spawned agent's system prompt
4. The spawned agent includes these tags in its `publish_report` call
5. The notification bus matches tags against subscriber subscriptions

### Job Persistence

Jobs are persisted to a JSON file by `JobWheel` and survive kernel restarts.
The wheel uses a `BTreeMap<(unix_seconds, Uuid), JobEntry>` for efficient
time-ordered retrieval. On each tick, `drain_expired()` yields fired jobs:

- **Once**: removed after firing
- **Interval**: `next_at` advanced by `every_secs`, re-inserted
- **Cron**: next fire time computed from expression, re-inserted

### In-Flight Recovery

When a job fires, it moves from the scheduling wheel to an **in-flight ledger**
(`in_flight.json`) before the execution agent is spawned. This handles the crash
window between drain and `publish_report`:

1. `drain_expired()` moves fired jobs to `in_flight` and persists both files
2. The kernel spawns an agent; the agent's manifest metadata carries the `job_id`
3. When the agent session ends (success or failure), `cleanup_process` calls
   `complete_in_flight(job_id)` to remove it from the ledger
4. On kernel restart, `take_in_flight()` returns any leftover in-flight jobs and
   they are re-fired as new agent sessions

For **recurring** jobs (Interval/Cron), both the rescheduled future entry and the
current in-flight copy are tracked — a crash only loses the current round, which
gets retried on restart.

### The Spawned Agent

When a scheduled job fires, the kernel spawns a dedicated agent session with:

- **Name**: `scheduled_job`
- **Role**: `Worker`
- **Max iterations**: 15
- **Max children**: 0 (no sub-spawning)
- **System prompt** that includes the task description, routing tags, and
  instructions to call `publish_report` when done
- **Metadata**: `{"scheduled_job_id": "<uuid>"}` for in-flight tracking

The spawned agent has access to all tools in the global `ToolRegistry`, including
the `kernel` syscall tool which enables it to publish its results.

## Background Tasks

### Spawning a Background Task

Any agent can spawn a background task using the `spawn-background` tool:

```json
{
  "manifest": {
    "name": "code-reviewer",
    "system_prompt": "You review pull requests for security issues.",
    "max_iterations": 10
  },
  "input": "Review PR #42 for security vulnerabilities",
  "description": "Security review of PR #42"
}
```

The background task runs as a child agent of the spawning session. The parent
receives a `BackgroundTaskStarted` stream event and can later cancel via
`cancel-background`.

### Differences from Scheduled Tasks

| Aspect | Scheduled Task | Background Task |
|--------|---------------|-----------------|
| Trigger | Time-based (delay/interval/cron) | Immediate |
| Persistence | Survives kernel restart | Lost on restart |
| Relationship | Independent session | Child of parent session |
| Cancellation | `schedule-remove` tool | `cancel-background` tool |
| Tags | Set at creation time | Set in publish_report call |

## Task Report & Notification Bus

### Publishing Results

When a task (scheduled or background) completes, the agent calls the `kernel`
tool with `action: "publish_report"`:

```json
{
  "action": "publish_report",
  "report": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000",
    "task_type": "pr_review",
    "tags": ["pr_review", "repo:rararulab/rara"],
    "status": "completed",
    "summary": "PR #42 approved — no issues found",
    "result": {
      "verdict": "approved",
      "confidence_score": 9,
      "risk_level": "low",
      "comments": []
    },
    "action_taken": "left approval comment on GitHub"
  }
}
```

**Fields:**

| Field | Required | Description |
|-------|----------|-------------|
| `task_id` | yes | Unique UUID for this task execution |
| `task_type` | yes | Category string (e.g. `"pr_review"`, `"deploy_check"`) |
| `tags` | yes | Routing tags — subscribers match against these |
| `status` | yes | `"completed"`, `"failed"`, or `"needs_approval"` |
| `summary` | yes | Human-readable one-line summary |
| `result` | yes | Task-type-specific structured JSON |
| `action_taken` | no | Description of action already taken, if any |
| `source_session` | no | Set automatically by kernel — agents should omit |

### Subscribing to Notifications

Any session can subscribe to task notifications by calling the `kernel` tool:

```json
{
  "action": "subscribe",
  "match_tags": ["pr_review", "critical"],
  "on_receive": "proactive_turn"
}
```

A subscription matches if **any** of its `match_tags` appears in the report's
`tags` array. Two delivery modes are available:

#### ProactiveTurn

Injects a synthetic user message into the subscriber's session using the
subscription owner's identity, triggering a full LLM turn. The message contains
the notification summary and structured result. Best for notifications that
require the subscriber to take action.

**Offline downgrade:** If the subscriber session is not currently alive in the
process table (e.g. after a kernel restart), delivery is automatically
downgraded to `SilentAppend` to avoid restoring the session with an incorrect
identity. The notification is written to tape so it is available when the
session is eventually restored.

#### SilentAppend

Appends the full `TaskNotification` to the subscriber's tape as a
`TapEntryKind::TaskReport` entry. The subscriber will see it on their next
context load but no LLM turn is triggered. Best for logging and audit trails.

### Unsubscribing

```json
{
  "action": "unsubscribe",
  "subscription_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

Subscriptions are also automatically cleaned up when a session ends
(`SubscriptionRegistry::remove_session`).

### SubscriptionRegistry

The registry is **file-backed** (`subscriptions.json`), persisted on every
mutation (subscribe/unsubscribe/remove_session). Since sessions are persistent
in Rara (SessionKey survives restart), subscriptions must also survive restarts
so that notifications continue to route correctly after a kernel restart.

## End-to-End Example

Here's a complete flow for a PR review notification:

```text
1. User Session A subscribes:
   kernel { action: "subscribe", match_tags: ["pr_review"], on_receive: "proactive_turn" }
   → Returns subscription_id

2. User Session A schedules a recurring PR check:
   schedule-interval {
     interval_seconds: 300,
     message: "Check for new PRs on rararulab/rara and review them",
     tags: ["pr_review", "repo:rararulab/rara"]
   }
   → Returns job_id

3. 5 minutes later, the job fires:
   → Kernel spawns a scheduled_job agent session
   → Agent reviews PRs using available tools
   → Agent calls: kernel { action: "publish_report", report: { ... } }

4. Kernel processes the report:
   a. Persists result to JobResultStore (results/{job_id}/{epoch}.json)
   b. Matches tags ["pr_review", "repo:rararulab/rara"] against subscriptions
   c. Session A's subscription matches on "pr_review"
   d. Session A is alive → delivers via ProactiveTurn (synthetic message)

5. Session A's LLM receives:
   [TaskNotification] pr_review: PR #42 approved — no issues found
   status: Completed
   result: {"verdict":"approved","confidence_score":9}
   action_taken: left approval comment on GitHub
   ref: <source_session>/entry_42

6. Session A can respond to the user or take further action.
```

## Relevant Source Files

| File | Purpose |
|------|---------|
| `crates/kernel/src/schedule.rs` | `JobWheel`, `JobEntry`, `Trigger` types |
| `crates/kernel/src/task_report.rs` | `TaskReport`, `TaskReportStatus`, domain result types |
| `crates/kernel/src/notification/` | `TaskNotification`, `SubscriptionRegistry`, `NotifyAction`, `NotificationBus` |
| `crates/kernel/src/syscall.rs` | `handle_publish_task_report()`, subscription dispatch |
| `crates/kernel/src/kernel.rs` | `handle_scheduled_task()` — agent spawning and system prompt |
| `crates/kernel/src/tool/schedule.rs` | `schedule-once/interval/cron/remove/list` tools |
| `crates/kernel/src/tool/spawn_background.rs` | `spawn-background` tool |
| `crates/kernel/src/tool/cancel_background.rs` | `cancel-background` tool |
