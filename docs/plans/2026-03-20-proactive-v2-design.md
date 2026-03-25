# Proactive V2 — Event-Driven Proactive Architecture

## Problem

Rara's current proactive mechanism is insufficient:

1. **Polling-only** — Mita heartbeat fires every 30 minutes, no real-time event response
2. **No signal sources** — Mita can only see process table and tape, blind to external changes
3. **Vague strategy** — Heartbeat message is "Analyze active sessions...", LLM freewheels every time
4. **No memory** — Mita doesn't remember past decisions, starts from zero each heartbeat

## Design

### Architecture

```
Internal/External Signals
    │
    ▼
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│ Event Source │────▶│ Event Filter │────▶│    Mita      │
│  (signals)   │     │ (rule-based)  │     │ (orchestrate)│
└─────────────┘     └──────────────┘     └──────┬──────┘
                                                │ dispatch_rara / notify
                                                ▼
                                         ┌─────────────┐
                                         │    Rara      │
                                         │ (user-facing) │
                                         └──────┬──────┘
                                                │ user reactions in tape
                                                ▼
                                         ┌─────────────┐
                                         │  Tape/Soul   │
                                         │ (feedback)    │
                                         └─────────────┘
```

Three layers:

- **Event Source** — Produces structured events via existing `KernelEvent` pipeline
- **Event Filter** — Pure rules, no LLM: quiet hours, cooldowns, rate limiting
- **Mita Orchestration** — Receives filtered events + context pack, decides action via existing tools

Heartbeat demoted from "sole driver" to "fallback patrol" — catches anything events missed.

### Role Separation

- **Rara** = front-stage, all user-facing communication (including proactive outreach)
- **Mita** = back-stage orchestrator, decides when/what Rara should do, never talks to user directly

Feedback loop: Rara delivers proactive message → user reacts → reaction captured in tape → Mita reads tape on next cycle to calibrate.

## Event Source Layer

### KernelEvent Extension

Proactive signals are new `KernelEvent` variants, not a separate event system:

```rust
pub enum KernelEvent {
    // ... existing variants ...

    /// Proactive signal for Mita orchestration.
    ProactiveSignal(ProactiveSignal),
}

pub enum ProactiveSignal {
    // --- Internal session events (Phase 1) ---
    /// Session has been idle beyond threshold.
    SessionIdle { idle_duration: Duration },
    /// Scheduled task agent failed.
    TaskFailed { error: String },
    /// Conversation naturally completed.
    SessionCompleted { summary: String },

    // --- Time events (Phase 1) ---
    /// Daily morning greeting trigger.
    MorningGreeting,
    /// End-of-day summary trigger.
    DailySummary,

    // --- GitHub events (Phase 2) ---
    /// PR review requested.
    PrReviewRequested { repo: String, pr_number: u64 },
    /// CI status changed.
    CiStatusChanged { repo: String, status: String },
    /// Issue assigned.
    IssueAssigned { repo: String, issue_number: u64 },
}
```

### Signal Emit Points (Phase 1)

**Session events:**

| Signal | Emit location | Trigger condition |
|--------|--------------|-------------------|
| `SessionIdle` | `IdleCheck` handler | Session idle beyond threshold; emit alongside existing Ready→Suspended transition |
| `TaskFailed` | `handle_scheduled_task` error path | ScheduledJobAgent execution failure |
| `SessionCompleted` | `TurnCompleted` handler | Conversation naturally ends |

**Time events:**

| Signal | Emit location | Trigger condition |
|--------|--------------|-------------------|
| `MorningGreeting` | Processor 0 scheduler | Work hours start (from proactive.yaml) |
| `DailySummary` | Processor 0 scheduler | Work hours end (from proactive.yaml) |

Time events: Processor 0 already computes `min(next_mita_heartbeat, next_job_deadline)`. Add `next_proactive_time_event` to the calculation. No new threads or processors.

## Event Filter Layer

Pure rule-based, zero LLM cost. Runs in `handle_event` after matching `ProactiveSignal`, before forwarding to Mita.

```rust
pub struct ProactiveFilter {
    /// Quiet hours — suppress all proactive signals.
    quiet_hours: Option<(NaiveTime, NaiveTime)>,
    /// Per-signal-kind minimum interval (dedup).
    cooldowns: HashMap<String, Duration>,
    /// Last fire time per signal kind.
    last_fired: HashMap<String, Timestamp>,
    /// Global rate limit.
    max_hourly: u32,
    /// Hourly counter.
    hourly_count: u32,
    hourly_window_start: Timestamp,
}

impl ProactiveFilter {
    /// Returns true = pass through, false = silently drop.
    pub fn should_pass(&mut self, signal: &ProactiveSignal) -> bool {
        // 1. Quiet hours check
        // 2. Cooldown dedup (e.g., SessionIdle same session max once per hour)
        // 3. Global hourly rate limit
    }
}
```

### Configuration

Stored at `config_dir()/mita/proactive.yaml` — this is Mita's own runtime config, not app config:

```yaml
# ~/.config/rara/mita/proactive.yaml
quiet_hours: ["23:00", "08:00"]
cooldowns:
  session_idle: 1h
  daily_summary: 20h
  morning_greeting: 20h
  task_failed: 10m
max_hourly: 5
work_hours:
  start: "09:00"
  end: "18:00"
  timezone: "Asia/Shanghai"
```

Mita can hot-update this config via a new `mita_update_proactive_config` tool — e.g., user says "weekends don't bother me", Mita adjusts quiet_hours.

## Mita Context Pack

When a filtered event reaches Mita, it's delivered as a structured message (replacing the current one-line "Analyze active sessions..."):

```
[Proactive Event]
kind: session_idle
timestamp: 2026-03-20T14:30:00Z

[Context]
session: "PR review discussion" (session-abc123)
idle_since: 2h ago
last_user_message: "我看看那个 PR"
user_status: online (last seen 10m ago)

[Mita History]
last_action: 4h ago — dispatched daily summary → user replied positively
last_action_on_this_session: none

[Available Actions]
- dispatch_rara: send a message to user through this session
- notify: push notification to user's device
- (no action): decide this event doesn't need intervention
```

Heartbeat patrol also uses this format with `kind: heartbeat_patrol`, context containing process table summary and changes since last patrol.

## Change Scope

### New files

- `kernel/src/proactive/signal.rs` — `ProactiveSignal` enum + context pack builder
- `kernel/src/proactive/filter.rs` — `ProactiveFilter` rule engine
- `kernel/src/proactive/config.rs` — Load from `config_dir()/mita/proactive.yaml`
- `kernel/src/proactive/mod.rs` — Re-exports (existing `proactive.rs` moves into directory)
- `app/src/tools/mita_update_proactive_config.rs` — Mita tool to adjust filter config

### Modified files

- `kernel/src/event.rs` — Add `ProactiveSignal` variant to `KernelEvent`
- `kernel/src/kernel.rs` scheduler — Add time event trigger points (morning/daily)
- `kernel/src/kernel.rs` `handle_event` — Match `ProactiveSignal`, run filter → build context → deliver to Mita
- `kernel/src/kernel.rs` `IdleCheck` handler — Emit `SessionIdle` signal
- `kernel/src/kernel.rs` `handle_mita_heartbeat` — Reformat as structured context pack

### Unchanged

- Mita's existing tool interface (dispatch_rara, notify, read_tape, etc.)
- JobWheel / ScheduledTask system
- Event queue / Processor architecture
- GroupPolicy / ProactiveJudgment (group chat judgment)

## Phases

| Phase | Scope | External deps |
|-------|-------|--------------|
| 1 | Internal session events + time events + filter + context pack | None |
| 2 | GitHub signals (via Symphony poll) | GitHub API (already integrated) |
| 3 | Generic webhook endpoint | HTTP listener |

Estimated Phase 1: ~500 lines new code, 6-8 files touched.

## Addendum (2026-03-21)

Additions made during Phase 1 implementation that extend the original design:

- **LLM judgment layer** — Optional lightweight LLM pre-filter (`signal_judgment.rs`) between `ProactiveFilter` and Mita delivery. Uses a cheap model (configured via `judgment_model` in `ProactiveConfig`) to decide whether a signal warrants Mita's attention. Skipped when `judgment_model` is absent.
- **Mita History in context pack** — `[Mita History]` section now populated from tape, showing Mita's recent proactive actions and user reactions for calibration.
- **SessionCompleted changed to idle-based** — `SessionCompleted` now fires after `session_completed_secs` (~10min) of session inactivity, not on turn completion. This avoids false positives from multi-turn conversations.
- **`mita_update_proactive_config` tool** — Mita can hot-update `ProactiveConfig` at runtime (e.g., user says "don't bother me on weekends").
- **Dynamic available actions** — `[Available Actions]` section in context pack is now built dynamically from Mita's actual tool registry instead of being hardcoded.
