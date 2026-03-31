<p align="center">
  <img src="site/public/favicon.svg" width="80" height="80" alt="rara">
</p>

<h1 align="center">rara</h1>

<p align="center">
  <em>your agent, harnessed by kernel</em>
</p>

<p align="center">
  <a href="https://rararulab.github.io/rara">Website</a> &middot;
  <a href="#getting-started">Getting Started</a> &middot;
  <a href="https://tape.systems">Tape Systems</a>
</p>

---

Think of an agent as a **process**. Rara is its **kernel**.

An operating system doesn't tell a process what to compute — it provides scheduling, memory, I/O, and protection. Rara does the same for agents: lifecycle, memory, tool access, channels, and guardrails. You define the behavior. Rara runs it.

## Highlights

- **Kernel architecture** — OS-inspired event loop: LLM, Tool, Memory, Session, Guard, EventBus
- **Tape memory** — Append-only fact model with anchors, handoffs, and sessions ([tape.systems](https://tape.systems))
- **Proactive** — Heartbeat-driven background actions, not just request-response
- **Multi-channel** — Web, Telegram, WeChat — one agent, many I/O surfaces
- **Skills** — Extensible capability system without touching core

## Getting Started

```bash
# prerequisites: rust, postgresql 17+, node 20+, just
just check        # install dependencies and verify
just run          # start the server
cd web && npm i && npm run dev  # start the frontend
```

Configuration lives in `~/.config/rara/config.yaml` — see `config.example.yaml`.

## License

Apache-2.0
