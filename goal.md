# rara — North Star

## What rara is

rara is a Rust implementation of the personal-AI-agent category
exemplified by [Hermes Agent](https://hermes-agent.nousresearch.com/):
a long-running local process that represents one user, accumulates memory
across years, and acts on its own initiative.

The bet: **Rust plus boring technology plus kernel discipline produces an
agent that runs for years without rewriting.** We trade time-to-feature
for time-to-decay.

## What rara is NOT

- **NOT a feature-parity race.** We will ship fewer integrations than Hermes
  if that is what the engineering bet costs. Single-surface depth comes
  before multi-surface breadth.
- **NOT a multi-user product.** rara learns one user's language, preferences,
  and rhythms. Multi-tenancy dilutes that signal to nothing.
- **NOT a code agent.** Claude Code and Cursor represent the developer's
  intent inside an IDE. rara represents the user's intent in their life
  and work — and acts on its own initiative, not on call.
- **NOT a black box.** Every decision rara makes must be inspectable through
  native eval interfaces. No "trust me" agents.
- **NOT a framework.** rara *is* one specific agent. We will not generalize
  into a library for spawning agents.

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

## Current focus (2026-Q2 — will rot)

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
