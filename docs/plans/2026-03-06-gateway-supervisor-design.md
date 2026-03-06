# Gateway Supervisor Design

> Issue: #85 — feat(core): add self-update mechanism for main branch deployments

## Design Decisions

- **Process model**: Single binary + `gateway` subcommand (`rara gateway` spawns `rara server`)
- **Update source**: Source-based (`git fetch` + `cargo build` in staging worktree)
- **Health check**: Stdout `READY` marker + HTTP `/health` polling

## Process Topology

```
rara gateway
  ├── SupervisorService    // 管理 agent 子进程生命周期
  │     ├── spawn(rara server)
  │     ├── health_check(stdout READY + HTTP /health)
  │     ├── restart(backoff policy)
  │     └── graceful_shutdown(SIGTERM → wait → SIGKILL)
  │
  ├── UpdateDetector       // 定期检查上游
  │     ├── git fetch origin main
  │     ├── compare HEAD vs origin/main
  │     └── notify if behind
  │
  └── UpdateExecutor       // 准备 + 激活新版本
        ├── git worktree add staging/<rev>
        ├── cargo build --release (in staging)
        ├── swap current binary
        ├── restart agent via Supervisor
        └── rollback if health check fails
```

## Configuration

New `gateway` section in YAML config:

```yaml
gateway:
  check_interval: 300        # seconds, upstream check interval
  health_timeout: 30         # seconds, health confirmation timeout
  health_poll_interval: 2    # seconds, HTTP poll interval
  max_restart_attempts: 3    # max consecutive restart failures
  auto_update: true          # whether to auto-apply updates
```

`staging_dir` is managed internally by `rara_paths` (e.g. `~/.local/share/rara/staging/`).

## Health Check Flow

```
Gateway spawn "rara server"
    │
    ├─ Phase 1: Wait for stdout "READY" marker
    │   └─ Timeout(health_timeout/2) → startup failure
    │
    └─ Phase 2: HTTP poll /health
        ├─ Every health_poll_interval seconds
        ├─ 3 consecutive 200 → confirmed healthy
        └─ Timeout(health_timeout/2) → startup failure
```

**Agent side change**: Print `READY` to stdout after HTTP/gRPC server bind succeeds.

## Restart Policy

```
Failure → wait 2s → retry
Again   → wait 4s → retry
Again   → wait 8s → retry (max_restart_attempts reached)
All failed → log error, stop retrying, Gateway stays alive for manual intervention
```

Exponential backoff. Counter resets after 60s of continuous healthy operation.

## Signal Propagation

Gateway receives SIGTERM/SIGINT → sends SIGTERM to child → waits 5s → SIGKILL if needed → Gateway exits.

## Source-based Update Flow

```
UpdateDetector (timed loop)
    │
    ├─ git fetch origin main
    ├─ Compare HEAD vs origin/main (git rev-parse)
    │
    └─ New commits?
         │
         ├─ auto_update=false → log "update available: {rev}"
         │
         └─ auto_update=true → trigger UpdateExecutor
              │
              ├─ 1. git worktree add ~/.local/share/rara/staging/<short-rev> origin/main
              ├─ 2. cargo build --release -p rara-cli (in staging worktree)
              ├─ 3. Build success?
              │     ├─ No → log error, clean staging, keep current version
              │     └─ Yes → continue
              ├─ 4. Copy new binary to staging directory
              ├─ 5. Replace current binary (rename original → .bak, rename new → in place)
              ├─ 6. Supervisor restarts Agent child process
              ├─ 7. Health confirmed?
              │     ├─ Yes → clean staging + .bak, log "updated to {rev}"
              │     └─ No → rollback: rename .bak back, restart Agent, clean staging
              └─ 8. git worktree remove staging
```

## Issue Breakdown

### Issue A: Gateway Supervision Foundation
- Add `rara gateway` subcommand to CLI
- `SupervisorService`: spawn / stop / restart `rara server` as child process
- Stdout `READY` marker wait + HTTP `/health` polling
- Exponential backoff restart policy
- SIGTERM/SIGINT signal propagation and graceful shutdown
- Agent side: print `READY` to stdout after server bind success
- `GatewayConfig` config section + YAML support

### Issue B: Update Detection
- `UpdateDetector`: timed `git fetch` + rev comparison
- Log detection results
- `check_interval` config driven
- Expose state: current rev / upstream rev / last check time

### Issue C: Update Preparation, Activation & Rollback
- `UpdateExecutor`: staging worktree → cargo build → binary replacement
- Integration with Supervisor: trigger restart + health confirmation
- Rollback logic: `.bak` restore + restart old version
- Staging cleanup

### Dependencies
- A is independent
- B and C depend on A (need Supervisor and config foundation)
- B and C are independent of each other → can be parallelized after A
