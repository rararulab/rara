# rara — North Star

## What rara is

rara is a Rust implementation of the personal-AI-agent category
exemplified by [Hermes Agent](https://hermes-agent.nousresearch.com/):
a long-running local process that represents one user, accumulates memory
across years, and acts on its own initiative.

rara also serves as this user's **autonomous trading agent.** It observes
markets, forms its own decisions, and acts on the user's behalf to execute
trades — always **within hard, user-set risk boundaries.** This is not a
tool the user drives trade-by-trade; it is an agent the user delegates to,
inside limits it may not cross.

The bet: **Rust plus boring technology plus kernel discipline produces an
agent that runs for years without rewriting — and trades for years without
blowing up or going out of control.** We trade time-to-feature for
time-to-decay, and we trade raw returns for surviving the tail.

## What rara is NOT

- **NOT a feature-parity race.** We will ship fewer integrations than Hermes
  if that is what the engineering bet costs. Single-surface depth comes
  before multi-surface breadth. In trading terms: one exchange, one asset
  class, proven end-to-end before we add breadth.
- **NOT a multi-user product.** rara learns one user's language, preferences,
  and rhythms. Multi-tenancy dilutes that signal to nothing.
- **NOT a code agent.** Claude Code and Cursor represent the developer's
  intent inside an IDE. rara represents the user's intent in their life
  and work — and acts on its own initiative, not on call.
- **NOT a black box** — and for trading this is a **hard constraint**, not a
  preference. Every decision rara makes must be inspectable through native
  eval interfaces, and **every trade must be a replayable trace**: which
  signals and factors drove it, what risk it took on, why it entered and why
  it exited. No unexplained fills. No "trust me" agents.
- **NOT a quant platform for others.** rara *is* one specific agent. Its
  factor and signal library exists to serve *rara's own* trading decisions —
  not as a third-party research or strategy-incubation framework. We will
  not generalize into a library for spawning agents or strategies.

## Safety invariants (trading)

Autonomous execution earns a set of invariants that the personal-agent
side does not need. These are specific to ③ (self-directed execution) and
are **code-enforced, not advisory** — they live in the type system and the
tests, not in a prompt or a README:

1. **Hard risk boundaries are first-class.** Position caps, per-trade and
   total exposure caps, and a max-drawdown kill-switch are user-set,
   enforced in code, and covered by tests. They are not knobs rara may talk
   itself past.
2. **Simulation first.** Paper-trading and backtest precede live. Real-money
   execution sits behind a switch the user must **explicitly arm**; the
   default state is simulated.
3. **Human-in-the-loop gate.** Any action beyond its pre-authorized
   boundaries stops and waits for a human — it does not widen its own
   mandate.
4. **Fail safe.** On uncertainty, error, or disconnect, rara **stops
   trading** — it holds and does nothing rather than guessing. Doing nothing
   is always a valid, safe action.

## What working rara looks like

Observable signals that the engineering bet is paying off:

1. **The process runs for months without intervention.** Memory does not
   grow unboundedly, file descriptors do not leak, internal state recovers
   without supervisor restarts.
2. **The user stops asking.** They no longer say "rara, do you remember X?"
   They expect rara to surface the right thing at the right time, unprompted.
3. **rara builds tools for the user.** From observed patterns, rara generates
   new jobs and capabilities on its own. Example: it notices the user reviews
   stocks every Monday morning and, without being asked, builds a scheduled
   stock-analysis job.
4. **Every action is inspectable.** Each decision can be pulled from the eval
   interface as a raw trace, score, and replayable record. No "I don't know
   why it did that."
5. **Memory survives time.** Recall accuracy does not degrade as the corpus
   grows from weeks to months to years to decades.
6. **It trades live for months inside its risk boundaries.** The account is
   not blown, there are no runaway loops, and no single day wipes out a
   season of gains. Survival first.
7. **Every trade is inspectable.** The replayable-trace constraint is
   observed in practice: for any fill, the driving signals, the risk taken,
   and the entry/exit rationale can be pulled back out. No "I don't know why
   it bought that."
8. **It never crosses a user-set risk cap.** The position, exposure, and
   drawdown limits hold under real conditions — enforced in code and covered
   by tests, not merely intended.

## Current focus (2026-Q2 — will rot)

Trading capability is built in three layers, and the ordering is
load-bearing:

- **① Market perception** — ingest feeds, detect anomalies and black-swan
  conditions, and alert. rara *sees* the market.
- **② Decision support** — backtest, evaluate, and advise. rara *forms and
  defends a view*, but the human still acts.
- **③ Autonomous execution** — rara places its own orders, real money,
  inside its risk boundaries. rara *acts*.

**③ is the end state, not the near term.** Live real-money execution is
hard-gated behind ① and ② being solid **plus** a stretch of paper-trading
track record — it does not ship until the safety invariants above are
enforced and the simulated record earns the arm switch.

The current focus (2026-Q2) is still **layer ① first**:

- Market perception — black-swan / anomaly alerting (issues 2415 / 2416 / 2417)
- Safety and stability hardening
- Performance
- Agent eval infrastructure
- The agent harness this document is part of

## How to use this document

This document gates spec-author. When drafting a contract for any feature,
change, or cleanup, spec-author MUST answer:

1. Which **"What working rara looks like"** signal does this advance?
   If none — reject the request, or update this document explicitly.
2. Does this cross a **"What rara is NOT"** line?
   If yes — reject the request, or update this document explicitly.
3. Does Hermes Agent already do this well, and do we have an engineering
   reason to do it differently? If no to both — strongly consider whether
   this work belongs in rara at all.

Either question being unclear is grounds for asking the user, not for
proceeding.

**Default-deny tiebreak.** When multiple signals overlap, when "Hermes does
this and we have a weak engineering reason", or when the answers feel
genuinely close — reject and surface the ambiguity to the user. It is cheap
to redo a rejection. It is expensive to ship the wrong thing.
