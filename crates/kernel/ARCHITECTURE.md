# kernel — Architecture

> A bird's-eye map of the `rara-kernel` crate, in the spirit of
> [matklad's ARCHITECTURE.md](https://matklad.github.io/2021/02/06/ARCHITECTURE.md.html).
> For crate-level invariants and anti-patterns see
> [`AGENT.md`](./AGENT.md). For per-subsystem deep dives, follow the
> `AGENT.md` files referenced inline below.

## Mission

`rara-kernel` is an OS-inspired event loop that coordinates LLM agent
"processes". Channel adapters (Telegram, web, CLI, STT, ...) publish
inbound messages onto a shared event queue; event processors drain
that queue, dispatch each event to a typed handler, and drive a
session through its lifecycle — loading tape memory, calling the LLM,
invoking tools, and delivering responses back out through the I/O
subsystem. The kernel owns no agent business logic of its own; it is
a scheduler, a resource gate, and an integration surface for the
subsystems listed below.

## Entry Point

The single public entry point is
[`Kernel::start`](./src/kernel.rs) at `src/kernel.rs:373`:

```rust
pub fn start(
    self,
    cancel_token: CancellationToken,
) -> (Arc<Self>, crate::handle::KernelHandle) { ... }
```

`start` consumes the assembled `Kernel`, wraps it in `Arc`, and spawns
the unified event loop via `Kernel::run` (`src/kernel.rs:404`). `run`
spawns one global processor plus `N` shard processors, each executing
`Kernel::run_processor` (`src/kernel.rs:454`). Only processor `id=0`
also runs the unified scheduler (Mita heartbeat + job wheel + rate
limiter GC).

External callers interact exclusively through the returned
[`KernelHandle`](./src/handle.rs) — mutations flow through the event
queue, so there is no second "control plane" to reason about.

## Data Flow — the Happy Path

A single bullet path covers the majority of traffic. Every arrow is a
real file you can grep for:

- **channel adapters** ([`src/channel/adapter.rs`](./src/channel/adapter.rs))
  receive raw platform events and construct
  `InboundMessage<Unresolved>`.
- **`IngressPipeline`** ([`src/io.rs:1308`](./src/io.rs)) resolves
  identity, applies rate limiting, and pushes a
  `KernelEventEnvelope::UserMessage` (`src/event.rs:274`) onto the
  queue.
- **`EventQueue`** / `ShardedQueue`
  ([`src/queue/mod.rs`](./src/queue/mod.rs),
  [`src/queue/sharded.rs`](./src/queue/sharded.rs)) buffers events
  per shard with back-pressure.
- **`run_processor`** (`src/kernel.rs:454`) drains up to 32 events at a
  time and spawns `handle_event` (`src/kernel.rs:600`) as a short-lived
  tokio task per event.
- **`handle_event`** dispatches to a typed handler — e.g.
  `handle_user_message` (`src/kernel.rs:1173`),
  `handle_group_message` (`src/kernel.rs:1468`),
  `handle_spawn_agent` (`src/kernel.rs:708`),
  `handle_signal` (`src/kernel.rs:835`),
  `handle_scheduled_task` (`src/kernel.rs:1361`),
  `handle_mita_heartbeat` (`src/kernel.rs:1625`),
  `handle_turn_completed` (`src/kernel.rs:2475`).
- Each handler calls into the agent run loop
  ([`src/agent/mod.rs`](./src/agent/mod.rs)) which interleaves
  **tape memory** writes ([`src/memory/`](./src/memory)) with LLM
  calls ([`src/llm/`](./src/llm)) and tool invocations
  ([`src/tool/`](./src/tool)).
- Output is delivered back out via
  **`IOSubsystem::deliver`** ([`src/io.rs`](./src/io.rs)) which routes
  to the originating channel adapter.

Other event kinds (child completion, signals, scheduled tasks) reuse
the same queue → processor → handler path.

## Module Taxonomy

The `src/` tree has 27 top-level modules across three tiers:

### Core — the kernel proper

These define the event loop, the agent process abstraction, and the
syscall / handle surface. Touching them means touching the scheduler.

| Module | Path | Role |
|---|---|---|
| kernel | [`src/kernel.rs`](./src/kernel.rs) | `Kernel` struct, event loop, handlers |
| event | [`src/event.rs`](./src/event.rs) | `KernelEventEnvelope` + event kinds |
| queue | [`src/queue/`](./src/queue) | `ShardedQueue`, per-shard `ShardQueue` |
| session | [`src/session/`](./src/session) | `SessionKey`, `SessionState`, registry |
| agent | [`src/agent/`](./src/agent) | Agent run loop, fold, loop-breaker, repetition guard |
| io | [`src/io.rs`](./src/io.rs) | `IngressPipeline`, `IOSubsystem::deliver`, rate limiter |
| syscall | [`src/syscall.rs`](./src/syscall.rs) | Syscall surface invoked by agents |
| handle | [`src/handle.rs`](./src/handle.rs) | `KernelHandle` — the external API |
| security | [`src/security.rs`](./src/security.rs) | `Principal` resolution, permission checks |
| channel | [`src/channel/`](./src/channel) | Adapter trait + registry |

### Subsystems — state holders called by the kernel

These are assembled into `Kernel` at construction time and are
accessed through typed subsystem fields. They each own persistent
state but do not run their own event loop.

| Module | Path | Role |
|---|---|---|
| tool | [`src/tool/`](./src/tool) | Tool registry + dispatch |
| llm | [`src/llm/`](./src/llm) | LLM clients + streaming |
| memory | [`src/memory/`](./src/memory) | Tape memory (JSONL) + session index |
| guard | [`src/guard/`](./src/guard) | Guard pipeline — see [`src/guard/AGENT.md`](./src/guard/AGENT.md) |
| schedule | [`src/schedule.rs`](./src/schedule.rs) | Job wheel driving scheduled tasks |
| notification | [`src/notification/`](./src/notification) | User-facing notification fan-out |

### Leaves — peripheral features

Opt-in integrations and domain features. They depend on Core and
Subsystems but nothing depends on them, so they can be modified in
isolation.

`browser` ([`src/browser/AGENT.md`](./src/browser/AGENT.md)),
`stt` ([`src/stt/AGENT.md`](./src/stt/AGENT.md)),
`mood` ([`src/mood.rs`](./src/mood.rs)),
`proactive` ([`src/proactive.rs`](./src/proactive.rs)),
`plan` ([`src/plan.rs`](./src/plan.rs)),
`debug` ([`src/debug.rs`](./src/debug.rs)),
`cascade` ([`src/cascade.rs`](./src/cascade.rs)),
`trace` ([`src/trace.rs`](./src/trace.rs)),
`metrics` ([`src/metrics.rs`](./src/metrics.rs)),
`identity` ([`src/identity.rs`](./src/identity.rs)),
`kv` ([`src/kv.rs`](./src/kv.rs)),
`task_report` ([`src/task_report.rs`](./src/task_report.rs)),
`user_question` ([`src/user_question.rs`](./src/user_question.rs)).

## Concurrency Model

- **One unified event loop** with `1 + N` processors
  (`src/kernel.rs:404`). With `num_shards == 0` only the global
  processor runs — the sharded and single-queue modes share one code
  path.
- **Per-event handlers** are spawned as independent tokio tasks
  (`src/kernel.rs:507`); handlers do not block the drain loop.
- **Global semaphore** (`src/kernel.rs:166`, initialised at
  `src/kernel.rs:254`) caps total concurrent agent processes across
  all shards. `handle_spawn_agent` acquires a permit at
  `src/kernel.rs:730`; per-session child limits are enforced by a
  separate `child_semaphore` (`src/kernel.rs:791`).
- **Scheduler ticks** (Mita heartbeat, job wheel, rate-limiter GC)
  only run on processor `id=0` (`src/kernel.rs:513`), so there is
  exactly one authoritative source of wall-clock-driven work.

## State Machines

### Session lifecycle

Defined in [`src/session/mod.rs:206`](./src/session/mod.rs) as
`enum SessionState { Active, Ready, Suspended, Paused }`:

- `Ready` — idle, awaiting next message.
- `Active` — an agent turn is in flight (LLM call or tool call).
- `Suspended` — timed out; resources released, but can be resumed.
- `Paused` — manually paused; rejects incoming messages.

`SessionState::is_terminal()` always returns `false`
(`src/session/mod.rs:222`): sessions are never truly terminal — they
transition to `Suspended` instead. Transitions happen via
`SessionRegistry::set_state` (`src/session/mod.rs:526`).

## Key Invariants

1. **The event loop is the only writer of kernel state.** External
   callers mutate state by pushing events through `KernelHandle`;
   they never touch `Kernel` fields directly.
2. **Handlers never hold kernel state across await points that could
   block the drain loop.** Handlers are spawned as independent tasks
   (`src/kernel.rs:507`) so the processor can keep draining.
3. **Syscalls are queued and processed by the event loop.** Agents
   do not mutate session or memory state directly — they issue
   syscalls ([`src/syscall.rs`](./src/syscall.rs)) dispatched through
   the same handler machinery as external events.
4. **`Principal` is never fabricated.** Every `Principal` must come
   from `SecuritySubsystem::resolve_principal`
   ([`src/security.rs`](./src/security.rs)) or `Principal::from_user`.
   Hollow principals bypass permission checks and are a review-blocker.
5. **Processor `id=0` is the single owner of scheduler ticks.** Mita
   heartbeat, job wheel drain, and rate-limiter GC only fire on the
   global processor (`src/kernel.rs:513`).

## Scheduling

The scheduling subsystem drives timed and recurring jobs. All
scheduling logic lives in [`src/schedule.rs`](./src/schedule.rs).

### Data Model

- **`JobEntry`** (`src/schedule.rs:173`) — a single scheduled task:
  job ID, trigger, message text, session key, principal, tags.
- **`Trigger`** (`src/schedule.rs:109`) — when a job fires. Three
  variants: `Once { run_at }`, `Interval { anchor_at, every_secs,
  next_at }`, `Cron { expr, next_at }`.
- **`JobWheel`** (`src/schedule.rs:259`) — the scheduling data
  structure. A `BTreeMap<(i64, Uuid), JobEntry>` keyed by
  `(next_fire_secs, job_uuid)` so `drain_expired` can pop all entries
  up to `now` in O(k) where k = expired count. A sidecar
  `HashMap<JobId, WheelKey>` provides O(1) removal by ID.
- **`DrainResult`** (`src/schedule.rs:204`) — output of
  `drain_expired`: `fired` (jobs to dispatch) + `cron_expired` (dead
  cron jobs for user notification).

### Persistence

Three files under the kernel data directory:

| File | Content |
|---|---|
| `jobs.json` | All registered jobs in the wheel |
| `in_flight.json` | Jobs drained but not yet completed, as `InFlightEntry` with lease metadata |
| `results/{job_id}/{epoch}.json` | Per-execution result log (via `JobResultStore`) |

**Lease semantics**: Each in-flight entry carries `fired_at` and
`lease_deadline` (`src/schedule.rs:229`). The default lease is 300
seconds (`DEFAULT_LEASE_SECS`, `src/schedule.rs:221`). On recovery,
`take_in_flight` (`src/schedule.rs:563`) only re-fires entries whose
lease has not expired. Expired entries are logged and discarded,
preventing orphaned jobs from re-firing forever across restarts.

### Ownership

Processor `id=0` is the sole scheduler owner (`src/kernel.rs:513`).
No other processor touches the job wheel. This eliminates
concurrency concerns — the wheel is behind `&mut self`, not
`Arc<Mutex<_>>`.

### Recovery on Restart

1. `JobWheel::load_with_clock` (`src/schedule.rs:302`) restores
   `jobs.json` and `in_flight.json` from disk.
2. On the first scheduler tick, `take_in_flight` returns entries
   whose `lease_deadline > now` for re-firing. Expired entries are
   discarded with a warning log.
3. `complete_in_flight` (`src/schedule.rs:543`) removes entries
   individually as each agent session ends — the ledger is crash-safe
   because incomplete entries persist until explicitly cleared.

### Trigger Semantics

- **`Once`** — fires at `run_at`, then removed from the wheel. Not
  rescheduled.
- **`Interval`** — catch-up from `anchor_at`: next fire =
  `anchor_at + k * every_secs` for smallest k > now. Eliminates
  cumulative drift even after missed periods. Legacy entries without
  `anchor_at` are backfilled from `next_at` at load time.
- **`Cron`** — `next_cron_time` (`src/schedule.rs:654`) computes the
  next fire via the `cron` crate. If the expression yields no future
  time, the job is moved to `DrainResult::cron_expired` and removed.
  Registration also rejects impossible expressions upfront.

### Clock Trait for Testability

`Clock` (`src/schedule.rs:43`) abstracts wall-clock time.
`SystemClock` (`src/schedule.rs:53`) delegates to
`jiff::Timestamp::now()`. `FakeClock` (`src/schedule.rs:64`) can be
set or advanced manually, making scheduler tests fully deterministic
with no `tokio::time::sleep` or real wall-clock dependencies.

## See Also

- [`AGENT.md`](./AGENT.md) — crate-level invariants and anti-patterns
- [`src/guard/AGENT.md`](./src/guard/AGENT.md) — guard pipeline internals
- [`src/browser/AGENT.md`](./src/browser/AGENT.md) — browser tool integration
- [`src/stt/AGENT.md`](./src/stt/AGENT.md) — speech-to-text subsystem
