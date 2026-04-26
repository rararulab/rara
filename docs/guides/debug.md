# Debugging rara — Remote Backend + Local Frontend

The canonical dev setup today: **backend runs on `raratekiAir` (`10.0.0.183`), frontend runs locally**. This doc teaches agents how to reach the remote, where logs live, and how to test end-to-end without rebuilding anything locally.

## Topology

```
┌────────── your mac ──────────┐          ┌────────── 10.0.0.183 (raratekiAir) ──────────┐
│                              │          │                                              │
│  web/ (vite dev, :5173)  ────┼── /api ─►│  rara server  :25555  (HTTP + WS)            │
│  VITE_API_URL=http://...     │          │  rara server  :50051  (gRPC)                 │
│                              │          │  rara gateway :25556  (supervisor, loopback) │
│                              │          │  logdy UI     :8080   (live log viewer)      │
└──────────────────────────────┘          └──────────────────────────────────────────────┘
```

- Backend process on the remote: `target/debug/rara server`, spawned by `target/debug/rara gateway`, which is itself started via `just run` in a login shell (not launchd — it dies if the shell dies).
- `/api/*` HTTP **and** WebSocket are both proxied by vite (`web/vite.config.ts`, `ws: true`).

## SSH access

Use the `local-rara` alias:

```bash
ssh local-rara                # interactive
ssh local-rara "<cmd>"        # one-shot
```

User is `rara`, home is `/Users/rara`, repo is `~/code/rararulab/rara`. Never edit source on the remote — always work in a local worktree and push a PR.

## Start the frontend against remote backend

```bash
cd web
VITE_API_URL=http://10.0.0.183:25555 bun run dev
```

Open `http://localhost:5173`. The proxy forwards `/api` (REST + WS) to the remote — no CORS needed. You should see `[heartbeat] ✓ GET /api/health → 200` in the vite terminal.

Quick reachability sanity check before you bother starting vite:

```bash
curl -s --max-time 3 http://10.0.0.183:25555/api/health
# expect: {"service":"job","status":"healthy",...}
```

If the curl fails but ping works, the backend process is down — see [Backend lifecycle](#backend-lifecycle).

## Logs

**Primary log location on the remote:** `/Users/rara/Library/Logs/rara/`

- `job.YYYY-MM-DD-HH` — main app log (hourly rotation, structured JSON lines)
- `raraerr.YYYY-MM-DD-HH` — stderr / panic output

```bash
# tail the current hour's log
ssh local-rara 'tail -f "$(ls -t /Users/rara/Library/Logs/rara/job.* | head -1)"'

# grep recent errors across the last few rotations
ssh local-rara 'ls -t /Users/rara/Library/Logs/rara/job.* | head -3 | xargs grep -i error | tail -50'

# stderr (panics, startup failures)
ssh local-rara 'tail -n 200 "$(ls -t /Users/rara/Library/Logs/rara/raraerr.* | head -1)"'
```

**Live log UI (logdy):** `http://10.0.0.183:8080` — browser-based filterable viewer, fed by the `dev.rara.logdy.*` launchd jobs. Good for watching a reproduction in real time without tail/grep.

## Backend lifecycle

The backend is **not** a launchd service. It runs inside a shell on the remote, started with `just run` (which execs `rara-cli gateway`, which supervises `rara server`).

```bash
# is it alive?
ssh local-rara 'lsof -iTCP:25555 -sTCP:LISTEN'

# who started it
ssh local-rara 'pgrep -fl "rara (server|gateway)"'

# restart from scratch (only when you really need to)
ssh local-rara 'cd ~/code/rararulab/rara && pkill -f "target/debug/rara " ; nohup just run >/tmp/rara-run.log 2>&1 &'
```

Before restarting, **confirm with the user** — other people may be using the instance. Prefer reading logs first.

## Database & config on the remote

Paths are resolved via `rara_paths` — on macOS that means `dirs::config_dir()`
+ `dirs::data_local_dir()`, which both land under `Library/Application Support`.

- Config: `/Users/rara/Library/Application Support/rara/config.yaml` (read-only from your side — propose YAML changes as a PR against `config.example.yaml`)
- DB: `/Users/rara/Library/Application Support/rara/db/rara.db` (SQLite, with `-wal` / `-shm` siblings while open)

Inspect DB without copying it off:

```bash
ssh local-rara 'sqlite3 "/Users/rara/Library/Application Support/rara/db/rara.db" "<query>"'
```

Do NOT run migrations or schema-mutating SQL on the remote manually — migrations live in `crates/rara-model/migrations/` and apply on boot.

## Reproducing an API issue end-to-end

1. `curl` the endpoint directly against the remote to confirm the backend behavior, independent of the frontend:
   ```bash
   curl -sS -i http://10.0.0.183:25555/api/<path>
   ```
2. In parallel, tail the remote log to capture the matching trace:
   ```bash
   ssh local-rara 'tail -f "$(ls -t /Users/rara/Library/Logs/rara/job.* | head -1)"' | grep -i <keyword>
   ```
3. If the HTTP call is fine but the UI breaks, the bug is frontend-side — inspect the browser devtools Network + Console. The proxy rewrites only the URL host; request/response bodies are pass-through.

## WebSocket debugging

The vite proxy has `ws: true` so WS upgrades on `/api/*` flow through. If WS fails:

- Verify `VITE_API_URL` uses `http://` (not `ws://`) — vite infers the WS scheme from the HTTP target.
- Check the vite terminal for `[heartbeat] ✗` or proxy error lines.
- Direct test (no proxy): `websocat ws://10.0.0.183:25555/api/<ws-path>` — if this works but the proxied one doesn't, the issue is local.

## What NOT to do

- Do NOT edit files on the remote. Work in a local worktree and follow the [standard workflow](workflow.md).
- Do NOT `pkill` or restart the remote backend without telling the user — they may be mid-session.
- Do NOT copy `rara.db` off the remote for "quick inspection" — it contains live user data. Query it in place.
- Do NOT point `VITE_API_URL` at `https://` — the remote serves plain HTTP on `:25555`.
- Do NOT assume a local `cargo run` will reproduce the bug — configs and DB state on the remote differ from yours.
