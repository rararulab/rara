# Gateway Supervisor

Rara supports a **gateway-supervised** deployment model where a long-lived gateway process manages the agent server as a child process. This enables automatic health monitoring, restart-on-failure, and source-based self-updates.

## Process Topology

```
rara gateway
  ├── SupervisorService    — spawn / health-check / restart child
  ├── UpdateDetector       — periodic git fetch + rev comparison
  └── UpdateExecutor       — staging build, binary swap, rollback
```

The gateway does **not** run the kernel itself. It spawns `rara server` as a child process and supervises it.

## Quick Start

```bash
# Start in supervised mode (gateway manages the server)
rara gateway

# Start in standalone mode (no supervision)
rara server
```

Both modes are fully functional. Use `rara gateway` when you want automatic restarts and self-updates.

## Health Check

The gateway uses a two-phase health check to confirm the agent is ready:

```
Phase 1: stdout READY marker
  └─ Agent prints "READY" after HTTP + gRPC servers bind successfully
  └─ Timeout: health_timeout / 2

Phase 2: HTTP /health polling
  └─ GET http://127.0.0.1:{port}/api/health
  └─ 3 consecutive 200 responses = healthy
  └─ Timeout: health_timeout / 2
```

Phase 1 confirms the process started and initialized. Phase 2 confirms HTTP is actually accepting requests.

## Restart Policy

When the agent crashes or fails health checks:

| Attempt | Backoff |
|---------|---------|
| 1st     | 2s      |
| 2nd     | 4s      |
| 3rd     | 8s      |

- Exponential backoff: `2^attempt` seconds
- Max attempts controlled by `max_restart_attempts` (default: 3)
- After 60 seconds of continuous healthy operation, the failure counter resets
- If all attempts are exhausted, the gateway logs an error and stays alive for manual intervention

## Signal Handling

| Signal | Behavior |
|--------|----------|
| SIGTERM / SIGINT to gateway | Gateway sends SIGTERM to child → waits 5s → SIGKILL if needed → gateway exits |
| Child exits with status 0 | Gateway restarts the child (unexpected for a server, but safe) |
| Child exits with non-zero | Counted as a failure, triggers backoff restart |

## Self-Update Flow

When `auto_update` is enabled, the gateway periodically checks for upstream changes and applies them automatically.

### Update Detection

The `UpdateDetector` runs a background loop:

1. `git fetch origin main`
2. Compare `HEAD` vs `origin/main` using `git rev-parse`
3. If revisions differ → update is available
4. Publish state via internal `watch` channel

Detection results are logged:

```
INFO update available: abc1234 (current: def5678)
```

### Update Execution

When an update is detected and `auto_update` is true:

```
1. Prepare   → git worktree add <staging>/<rev> origin/main
2. Build     → cargo build --release -p rara-cli (in staging)
3. Activate  → rename current binary → .bak, copy new binary in place
4. Restart   → supervisor restarts agent child process
5. Verify    → two-phase health check
6. Finalize  → success: clean .bak + staging
               failure: rollback .bak, restart old version
```

Key properties:
- **Isolated build**: staging worktree is separate from the live runtime
- **Atomic swap**: binary replacement uses rename (atomic on same filesystem)
- **Safe rollback**: `.bak` file is the safety net, restored on health check failure
- **Build timeout**: 10 minutes max for `cargo build`

### Runtime Layout

```
~/.local/share/rara/
  └── staging/
        └── abc1234/     ← staging worktree (temporary)
```

The staging directory is managed internally by `rara_paths` and is not user-configurable.

## Admin API

The gateway exposes its own HTTP server (default port `25556`, separate from the agent's `25555`) for operational control.

### Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/gateway/status` | Agent status, current/upstream rev, update availability |
| `POST` | `/gateway/restart` | Gracefully restart the agent child process |
| `POST` | `/gateway/update` | Build the latest upstream revision, activate it, and restart |
| `POST` | `/gateway/shutdown` | Graceful shutdown of gateway + agent |

### `GET /gateway/status`

```json
{
  "agent": {
    "running": true,
    "restart_count": 0,
    "pid": 12345
  },
  "update": {
    "current_rev": "abc1234...",
    "upstream_rev": "def5678...",
    "update_available": true,
    "last_check_time": "2026-03-06T12:00:00Z"
  }
}
```

### `POST /gateway/restart`

Triggers a graceful restart of the agent: SIGTERM → wait → respawn. This is a **manual restart** — it does not count as a failure and does not trigger backoff.

```bash
curl -X POST http://127.0.0.1:25556/gateway/restart
# {"ok": true}
```

### `POST /gateway/update`

Triggers the same supervised update pipeline used by automatic updates: detect the latest upstream revision, build in a staging worktree, atomically swap the binary, verify health, and roll back on failure.

```bash
curl -X POST http://127.0.0.1:25556/gateway/update
# {"message":"Updated to <rev> and restarted successfully."}
```

### `POST /gateway/shutdown`

Gracefully shuts down both the agent and the gateway process.

```bash
curl -X POST http://127.0.0.1:25556/gateway/shutdown
# {"ok": true}
```

## Telegram Admin Commands

When the Telegram adapter is enabled, Rara also exposes two privileged bot commands backed by the gateway admin API:

| Command | Behavior | Authorization |
|---------|----------|---------------|
| `/restart` | Calls `POST /gateway/restart` and returns a short acknowledgement | Only the configured `telegram.chat_id` |
| `/update` | Calls `POST /gateway/update` and returns the gateway's summary message | Only the configured `telegram.chat_id` |

Operational requirements:
- The bot must run in a process started by `rara gateway`; in standalone `rara server` mode there is no gateway admin API to receive these requests.
- `gateway.bind_address` must point at the local gateway listener reachable from the app process.
- If no owner chat ID is configured, admin commands stay unavailable instead of failing open.
- `/update` is synchronous from Telegram's perspective and can take several minutes because it waits for fetch, build, restart, and health verification.

## Configuration

Add an optional `gateway` section to your YAML config:

```yaml
gateway:
  bind_address: "127.0.0.1:25556"  # admin API listen address
  check_interval: 300        # seconds between upstream checks
  health_timeout: 30         # total health confirmation timeout (seconds)
  health_poll_interval: 2    # HTTP poll interval (seconds)
  max_restart_attempts: 3    # max consecutive restart failures
  auto_update: true          # automatically apply upstream updates
```

All fields have sensible defaults. If the `gateway` section is omitted entirely, defaults are used when running `rara gateway`.

| Key | Default | Description |
|-----|---------|-------------|
| `bind_address` | `127.0.0.1:25556` | Admin API listen address |
| `check_interval` | `300` | How often to `git fetch` and check for updates (seconds) |
| `health_timeout` | `30` | Total time budget for both health check phases (seconds) |
| `health_poll_interval` | `2` | Interval between HTTP `/health` poll requests (seconds) |
| `max_restart_attempts` | `3` | Max consecutive failures before stopping restarts |
| `auto_update` | `true` | Whether to automatically build and activate updates |
