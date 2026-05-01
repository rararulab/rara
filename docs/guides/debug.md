# Debugging rara — Local-First Dev, Remote as Production

**Dev model:** rara runs **locally** for development and end-to-end
testing. The remote `raratekiAir` (`10.0.0.183`) instance is
**production** — only touch it when you need to reproduce a
production-only bug, inspect production data, or verify behavior under
the production config + DB. Day-to-day iteration (UI changes, API
changes, smoke tests, click-through verification) happens on your own
machine, not on the shared remote.

If you are about to report a UI or API change as complete and you have
not opened a browser against your local stack — start it now (see
[Local development](#local-development) below). That is the section the
"open the dev server before claiming a UI change is done" rule points
to.

## Local development (the default)

Two processes, both on your machine:

```bash
# terminal 1 — backend (gateway supervises rara server)
just run

# terminal 2 — frontend (vite, proxies /api to localhost:25555 by default)
cd web
bun run dev
```

Open `http://localhost:5173`. The vite proxy forwards `/api` (REST and
WebSocket; `web/vite.config.ts` has `ws: true`) to the local backend on
`:25555` — no CORS, no SSH, no shared state. You should see
`[heartbeat] ✓ GET /api/health → 200` in the vite terminal.

Quick reachability check:

```bash
curl -s --max-time 3 http://127.0.0.1:25555/api/health
# expect: {"service":"job","status":"healthy",...}
```

Local config and DB live under your user paths (`rara_paths` resolves
via `dirs::config_dir()` + `dirs::data_local_dir()` — on macOS that's
`~/Library/Application Support/rara/`). Migrations apply on boot from
`crates/rara-model/migrations/`; if the local DB is corrupted, run
`just migrate-reset`.

This is the loop you should be in for almost everything: code change →
`just run` (or let it auto-reload via the gateway) → `bun run dev` →
click through the affected path in the browser → check the local log.

## Topology

```
┌────────── your mac (development) ──────────┐
│  web/ (vite dev, :5173)  ──── /api ──►     │
│  bun run dev                               │
│  rara server  :25555 (HTTP + WS)           │
│  rara server  :50051 (gRPC)                │
│  rara gateway :25556 (supervisor, loopback)│
└────────────────────────────────────────────┘

┌────────── 10.0.0.183 raratekiAir (PRODUCTION) ──────────┐
│  rara server  :25555  (HTTP + WS)                       │
│  rara server  :50051  (gRPC)                            │
│  rara gateway :25556  (supervisor, loopback)            │
│  logdy UI     :8080   (live log viewer)                 │
└─────────────────────────────────────────────────────────┘
```

You only need the right-hand box when the bug is production-specific —
otherwise everything below in [Production debugging](#production-debugging)
is irrelevant to your task.

## Production debugging

Reach for the remote **only** when one of these is true:

- The bug reproduces against production but not against your local
  stack, and you need production logs / DB state to understand why.
- You are inspecting real user data the local DB does not have.
- You are verifying a fix under the actual production config + traffic
  before declaring it shipped.

If the symptom reproduces locally, debug locally — production is not
the dev backend.

### SSH access

Use the `local-rara` alias:

```bash
ssh local-rara                # interactive
ssh local-rara "<cmd>"        # one-shot
```

User is `rara`, home is `/Users/rara`, repo is `~/code/rararulab/rara`.
Never edit source on the remote — always work in a local worktree and
push a PR.

### Pointing your local frontend at production

Only when you specifically need to reproduce a production-only frontend
bug:

```bash
cd web
VITE_API_URL=http://10.0.0.183:25555 bun run dev
```

Open `http://localhost:5173`. The proxy forwards `/api` (REST + WS) to
the remote — no CORS needed.

Quick reachability sanity check before you bother starting vite:

```bash
curl -s --max-time 3 http://10.0.0.183:25555/api/health
# expect: {"service":"job","status":"healthy",...}
```

If the curl fails but ping works, the production backend process is
down — see [Backend lifecycle (production)](#backend-lifecycle-production).

### Logs (production)

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

**Live log UI (logdy):** `http://10.0.0.183:8080` — browser-based
filterable viewer, fed by the `dev.rara.logdy.*` launchd jobs. Good
for watching a reproduction in real time without tail/grep.

### Backend lifecycle (production)

The production backend is **not** a launchd service. It runs inside a
shell on the remote, started with `just run` (which execs `rara-cli
gateway`, which supervises `rara server`).

```bash
# is it alive?
ssh local-rara 'lsof -iTCP:25555 -sTCP:LISTEN'

# who started it
ssh local-rara 'pgrep -fl "rara (server|gateway)"'

# restart from scratch (only when you really need to)
ssh local-rara 'cd ~/code/rararulab/rara && pkill -f "target/debug/rara " ; nohup just run >/tmp/rara-run.log 2>&1 &'
```

Restarting production is gate (c) in the workflow — **confirm with the
user before `pkill`-ing or restarting**, because other people may be
using the instance. Prefer reading logs first; restarts are rare.

### Database & config (production)

Paths are resolved via `rara_paths` — on macOS that means
`dirs::config_dir()` + `dirs::data_local_dir()`, which both land under
`Library/Application Support`.

- Config: `/Users/rara/Library/Application Support/rara/config.yaml`
  (read-only from your side — propose YAML changes as a PR against
  `config.example.yaml`)
- DB: `/Users/rara/Library/Application Support/rara/db/rara.db`
  (SQLite, with `-wal` / `-shm` siblings while open)

Inspect DB without copying it off:

```bash
ssh local-rara 'sqlite3 "/Users/rara/Library/Application Support/rara/db/rara.db" "<query>"'
```

Do NOT run migrations or schema-mutating SQL on the remote manually —
migrations live in `crates/rara-model/migrations/` and apply on boot.

### Reproducing a production API issue end-to-end

1. `curl` the endpoint directly against the remote to confirm the
   backend behavior, independent of the frontend:
   ```bash
   curl -sS -i http://10.0.0.183:25555/api/<path>
   ```
2. In parallel, tail the remote log to capture the matching trace:
   ```bash
   ssh local-rara 'tail -f "$(ls -t /Users/rara/Library/Logs/rara/job.* | head -1)"' | grep -i <keyword>
   ```
3. If the HTTP call is fine but the UI breaks, the bug is frontend-side
   — inspect the browser devtools Network + Console. The proxy rewrites
   only the URL host; request/response bodies are pass-through.

### WebSocket debugging

The vite proxy has `ws: true` so WS upgrades on `/api/*` flow through.
If WS fails:

- Verify `VITE_API_URL` uses `http://` (not `ws://`) — vite infers the
  WS scheme from the HTTP target.
- Check the vite terminal for `[heartbeat] ✗` or proxy error lines.
- Direct test (no proxy): `websocat ws://10.0.0.183:25555/api/<ws-path>`
  — if this works but the proxied one doesn't, the issue is local.

(For local WS debugging, swap `10.0.0.183` for `127.0.0.1`.)

## What NOT to do

- Do NOT use the remote as your dev backend. It is production. Run
  rara locally for development and end-to-end testing — that is
  exactly why dev/prod are separated.
- Do NOT report a UI or API change as complete without opening a
  browser against your local stack and exercising the affected path.
  `cargo test` + vitest do not catch render bugs, wiring bugs, or
  proxy bugs.
- Do NOT edit files on the remote. Work in a local worktree and follow
  the [standard workflow](workflow.md).
- Do NOT `pkill` or restart the production backend without telling the
  user — gate (c) in the workflow. Other people may be mid-session.
- Do NOT copy `rara.db` off the remote for "quick inspection" — it
  contains live user data. Query it in place.
- Do NOT point `VITE_API_URL` at `https://` — the remote serves plain
  HTTP on `:25555`.
