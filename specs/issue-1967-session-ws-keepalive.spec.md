spec: task
name: "issue-1967-session-ws-keepalive"
inherits: project
tags: [bug, channels, websocket]
---

## Intent

The persistent per-session WebSocket handler in `crates/channels/src/web_session.rs`
has no server-side keepalive. The `handle_session_ws` task spawns three forwarders
plus a `send_task` and a `recv_task` that `select!` on each other and a shutdown
signal — there is no `tokio::time::interval` driving periodic pings, and no `Pong`
frames are emitted in response to client pings either. `axum::extract::ws::WebSocket`
does NOT auto-ping by default; long-lived sockets that carry no application data
during an idle period therefore have no traffic on the wire.

If we do not do this, the following concrete bug appears. Reproducer:

1. Backend runs on `raratekiAir` (`10.0.0.183:25555`); the frontend runs locally
   at `http://localhost:5173` with `localStorage.rara_backend_url=http://10.0.0.183:25555`,
   bypassing the Vite proxy so the browser opens the WS straight at the LAN host.
2. The user connects, idles (no prompt sent), and waits.
3. After 30s–5min — well below any explicit server timeout — the connection is
   reaped by an intermediate hop (Firefox tab throttling, NAT mapping in
   `10.0.0.0/8` routers, intermediate firewall). Server log
   (`/Users/rara/Library/Logs/rara/job.*` on `local-rara`) shows
   `persistent session WS auth via owner token` → `response status=101` →
   `persistent session WS closed` cycles at 04:11:22→04:12:04 (42s) and
   04:12:14→04:16:49 (4.5min). The frontend then logs
   `Firefox can't establish a connection to ws://10.0.0.183:25555/api/session/...`
   on every reconnect attempt because the new connection races the same fate.

The fix landed in PR 1883 (issue 1880) added bounded-backoff auto-reconnect on
the **frontend** — that addresses UI failure modes when a drop has already
happened, but does not prevent the drop. The drop itself is what produces the
console spam and the burned reconnect budget. Server-side ping at a fixed
interval (well below typical NAT-reap windows) keeps the wire warm and the
mapping alive.

`grep -n "ping\|pong\|keepalive\|interval\|heartbeat" crates/channels/src/web_session.rs`
returns nothing — confirmed greenfield gap. PR 1935 (the original scaffold)
never wired keepalive; PR 1883 only touched the frontend. No prior commit
removed server-side WS keepalive, so this is not a regression-decision reversal.

This advances `goal.md` signal 1 ("The process runs for months without
intervention") — a WS that survives idle periods is part of "runs for months"
on real LAN topologies, not just localhost dev.

## Decisions

- Server emits `Message::Ping` on a fixed interval from inside the existing
  `send_task`. Interval is a Rust `const` next to `handle_session_ws` (mechanism
  tuning, not deploy-relevant — see `docs/guides/anti-patterns.md` "Mechanism
  constants are not config"). Initial value: 30 seconds.
- The recv loop already accepts `Message::Pong` implicitly via the existing
  `_ => continue` arm; no extra handling required for client-initiated pongs.
- Explicit handling for client-initiated `Message::Ping` is not needed — `axum`
  / `tungstenite` automatically replies `Pong` to inbound `Ping` frames at the
  protocol layer. The change is strictly: server starts emitting pings.
- Inactivity-based disconnect (e.g. "close after N missed pongs") is OUT OF
  SCOPE. The bug is "wire goes silent → NAT kills mapping", not "client is
  unresponsive but socket pretends to be open". Adding a missed-pong watchdog
  would solve a different problem and risks regressions for slow-network users.

## Boundaries

### Allowed Changes
- `crates/channels/src/web_session.rs` — add periodic ping in `send_task`'s
  `tokio::select!` via a `tokio::time::interval`. Add a `const` for the
  interval next to the function.
- `crates/channels/tests/web_session_smoke.rs` — add the integration test
  bound to the scenarios below.
- `specs/issue-1967-session-ws-keepalive.spec.md`
- **/crates/channels/src/web_session.rs
- **/crates/channels/tests/web_session_smoke.rs
- **/specs/issue-1967-session-ws-keepalive.spec.md

### Forbidden
- Do NOT add a YAML config knob for the ping interval. Mechanism tuning is a
  Rust `const` — see `docs/guides/anti-patterns.md`.
- Do NOT add ping/keepalive to the legacy chat WS in `crates/channels/src/web.rs`.
  That endpoint is being phased out; PR 1935 already deleted the legacy split
  but kept some compat surface. Touching it widens scope.
- Do NOT add a missed-pong disconnect watchdog. See Decisions.
- Do NOT change the frontend (`web/src/agent/session-ws-client.ts` etc.) —
  PR 1883 already handles client-side reconnect. No frontend change is needed
  for keepalive itself, since browsers auto-reply `Pong` to `Ping` at the
  protocol layer.
- The keepalive interval must live in `send_task` so it composes with the
  existing `select!` and shutdown handling — adding a sibling `sleep`-loop
  task in any of the three forwarders is out of scope.

## Completion Criteria

Scenario: Server emits periodic Ping frames on an idle persistent session WS
  Test:
    Package: rara-channels
    Filter: session_ws_emits_periodic_ping_when_idle
  Given a persistent session WS connected and authenticated
    And the test overrides the ping interval to a short value (e.g. 100ms via test-only setter or a #[cfg(test)] const)
  When the client stays idle for at least three intervals
  Then the client receives at least two Ping frames from the server
    And the connection remains open

Scenario: Periodic ping does not interfere with normal event delivery
  Test:
    Package: rara-channels
    Filter: session_ws_ping_does_not_disturb_events
  Given a persistent session WS connected and authenticated
  When an adapter event is published while the ping interval is active
  Then the client receives the JSON event frame in order
    And ping frames continue to be emitted on schedule

## Out of Scope

- Frontend changes (PR 1883 already handles reconnect symptoms)
- Missed-pong disconnect watchdog
- Keepalive on the legacy `crates/channels/src/web.rs` chat WS
- A YAML knob for ping interval
- Changes to the gateway / supervisor process
