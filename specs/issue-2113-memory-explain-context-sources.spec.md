spec: task
name: "issue-2113-memory-explain-context-sources"
inherits: project
tags: []
---

## Intent

Issues 2111 and 2112 give rara a confidence-feedback loop on the
knowledge layer. The missing third piece — borrowed from AMFS's
`explain()` — is the *trace from a model turn back to the memory items
that shaped its context*. Without it, the feedback loop is half-blind:
when a tool result lands and we want to call `commit_outcome` on "the
items that fed this turn", we cannot, because we never recorded which
items were retrieved.

It also breaks goal signal 4 ("every action is inspectable") at
exactly the layer where opacity hurts the most: the user looks at a
weird answer and asks "why did rara think that?", and the only path
today is to re-run the query and hope the retrieval is deterministic.

Reproducer for the failure mode:

1. User asks "what dietary restrictions did I mention?". rara
   retrieves memory items {17, 42, 88} via embedding similarity,
   builds an LLM prompt that injects them, the model responds, the
   tape persists the assistant message.
2. The user later disputes the answer ("you got 88 wrong").
3. Today: there is no way to recover from tape what items rara
   pulled into that turn's context. The retrieval ids exist only in
   stack frames during the turn; once the function returns they are
   gone.
4. Concretely: `commit_outcome(&[?], Contradicted, …)` has no
   `?` to fill in. We end up either guessing (re-running retrieval
   and hoping it returns the same set) or doing nothing.

The fix is to persist the retrieval set as part of the tape, then
expose a thin `KnowledgeService::explain(tape_entry_id) -> Vec<(MemoryItem, f32)>`
API that reads it back.

Goal alignment: signal 4 ("every action is inspectable" — this is
literally the inspection seam), and it unblocks the
"automatic-feedback-from-outcomes" follow-up that closes the loop on
signals 2 and 5. Crosses no `NOT` line — single-user, inspectable,
not a framework.

Prior art reviewed (raw):

- `gh issue list --search "explain memory"` — 0 matching issues.
- `gh pr list --search "context sources memory"` — 0 PRs.
- `git log --grep=explain --since=180.days` — only unrelated hits
  (commit messages mentioning the word in prose).
- `rg -n "ContextSources" crates/` — 0 matches. Greenfield name.
- `crates/kernel/src/memory/codec.rs` and `crates/kernel/src/memory/`
  define `TapEntryKind` — confirmed via `context.rs` which matches
  on `Message | ToolCall | ToolResult | Event | System | Anchor |
  Note | Summary` (8 variants). Adding a ninth kind is the existing
  extension mechanism; no novelty in the approach.
- The retrieval path is centralized in
  `crates/kernel/src/memory/knowledge/tool.rs::exec_search` and
  `crates/kernel/src/memory/knowledge/extractor.rs` — finite set of
  call sites to record from.

## Decisions

- **Depends on issues 2111 and 2112** merged into `main`. The
  `MemoryItem.confidence` field that `explain()` returns alongside
  each item lands in 2111; the `commit_outcome` consumer that this
  trace exists to serve lands in 2112.
- **New `TapEntryKind::ContextSources`** variant with payload shape:
  ```
  { "item_ids": [17, 42, 88], "weights": [0.93, 0.81, 0.55] }
  ```
  `weights` are the per-item retrieval scores at the moment of the
  turn — for the embedding-search path that is
  `1.0 - distance` clamped to `[0.0, 1.0]`. Why include weights at
  all and not just ids: future `explain()` callers want a "how much
  did each item contribute" answer, not just a set; the weight is
  free at retrieval time and expensive to reconstruct later.
  Recording it now avoids a second migration when the UI surface
  needs it.
- **Per-turn emission contract**: `MemoryTool::exec_search` writes
  exactly one `ContextSources` tape entry per call, on the **current
  turn's tape**, before the search results return to the caller.
  Empty result sets still emit an entry (with empty arrays) so that
  `explain()` can distinguish "search ran and found nothing" from
  "search never ran".
- **Tape selection**: the existing `TapeService` already routes
  writes; the entry attaches to the same tape the calling turn is
  appending to. The implementer wires this through the
  `MemoryTool`'s existing access to the tape service (the tool is
  already kernel-resident; this is a service-locator decision,
  not a new dependency). If the tool does not already hold a
  `TapeService` reference, add it on construction — `MemoryTool` is
  built inside the kernel, no public API change leaks.
- **`KnowledgeService::explain` signature**:
  ```
  pub async fn explain(
      &self,
      tape_name: &str,
      turn_entry_id: i64,
  ) -> Result<Vec<(MemoryItem, f32)>>;
  ```
  Why two parameters: the kernel uses `(tape_name, entry_id)` as the
  identity of a tape entry — there is no globally unique
  `tape_entry_id`. (Verified against the tape store layout:
  `FileTapeStore` indexes by `(tape_name, entry_id)`.) Returning a
  pair `(item, weight)` mirrors the persisted shape. Items are
  returned in the order they appear in the tape entry's
  `item_ids` array — i.e. retrieval-rank order, not by id — so the
  caller can show the most-contributing memory first.
- **Resolution semantics**:
  - If no `ContextSources` entry exists for `(tape_name, turn_entry_id)`
    or any entry within the same turn-window, return `Ok(vec![])` —
    "the turn did not consult memory" is a valid answer.
    Concretely the implementer walks the tape from
    `turn_entry_id` forward until the next non-tool entry,
    collecting `ContextSources` entries; in practice there is
    zero or one.
  - If a `ContextSources` entry references an `item_id` that has
    been deleted since the turn, that id is skipped from the
    returned vec (no error). Rationale: matching `commit_outcome`'s
    skip-unknown-ids policy; explain is a debugging seam, not a
    consistency contract.
- **Do NOT add a new MCP tool** that exposes `explain` to the LLM.
  This is an internal Rust API; UI / API surface comes later.
- **Do NOT auto-call `commit_outcome` from `explain` results.** That
  wiring is a separate change; this PR is read-only on the
  confidence column.
- **`ContextSources` entries are recorded for ALL search calls**
  (not gated on a config flag). Per project spec, "mechanism
  defaults are always on"; adding a YAML toggle here is exactly the
  mechanism-vs-config anti-pattern. The cost is one extra tape
  append per `MemoryTool::search` action — negligible at JSONL
  write rates.
- **`weights` precision**: stored as `f32` after clamp; serialized
  to JSON via `serde_json` default. No rounding beyond IEEE-754
  representation. The downstream consumers do not depend on exact
  values, only ordering.

## Boundaries

### Allowed Changes
- crates/kernel/src/memory/codec.rs
- **/crates/kernel/src/memory/codec.rs
- crates/kernel/src/memory/mod.rs
- **/crates/kernel/src/memory/mod.rs
- crates/kernel/src/memory/knowledge/service.rs
- **/crates/kernel/src/memory/knowledge/service.rs
- crates/kernel/src/memory/knowledge/tool.rs
- **/crates/kernel/src/memory/knowledge/tool.rs
- crates/kernel/src/memory/knowledge/mod.rs
- **/crates/kernel/src/memory/knowledge/mod.rs
- crates/kernel/src/testing.rs
- **/crates/kernel/src/testing.rs
- crates/app/src/boot.rs
- **/crates/app/src/boot.rs
- specs/issue-2113-memory-explain-context-sources.spec.md
- **/specs/issue-2113-memory-explain-context-sources.spec.md

### Forbidden
- crates/rara-model/**
- crates/kernel/src/memory/knowledge/items.rs
- crates/kernel/src/memory/knowledge/extractor.rs
- crates/kernel/src/memory/knowledge/embedding.rs
- crates/kernel/src/memory/knowledge/outcome.rs
- crates/kernel/src/memory/knowledge/config.rs
- crates/kernel/src/memory/knowledge/categories.rs
- crates/kernel/src/memory/context.rs
- crates/kernel/src/memory/store.rs
- crates/kernel/src/memory/tree.rs
- crates/kernel/src/memory/anchors.rs
- config.example.yaml
- .github/workflows/**
- docs/**
- web/**

## Completion Criteria

Scenario: search emits a ContextSources tape entry with item ids and weights
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::tool::tests::search_emits_context_sources_entry
  Given a knowledge corpus with three memory items embedded and indexed
  When MemoryTool::exec_search runs for a query that matches all three
  Then exactly one ContextSources entry is appended to the active tape, its item_ids array equals the returned ids in order, and its weights array has the same length

Scenario: search with no matches still emits an empty ContextSources entry
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::tool::tests::search_emits_empty_context_sources_when_no_matches
  Given an empty knowledge corpus
  When MemoryTool::exec_search runs against any query
  Then a ContextSources entry exists on the tape with item_ids = [] and weights = []

Scenario: explain returns the memory items recorded for a turn in retrieval order
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::service::tests::explain_returns_items_in_retrieval_order
  Given a tape with a ContextSources entry referencing memory items {17, 42, 88} with weights {0.93, 0.81, 0.55}
  When KnowledgeService::explain runs against that tape and the entry id of the turn
  Then it returns the three MemoryItem rows paired with the weights in the same order

Scenario: explain returns an empty vec when the turn did not consult memory
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::service::tests::explain_returns_empty_for_turn_without_context_sources
  Given a tape with a Message turn entry but no ContextSources entry following it
  When KnowledgeService::explain runs against that turn's entry id
  Then it returns Ok with an empty vec

Scenario: explain skips item ids whose memory_items rows have been deleted
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::service::tests::explain_skips_deleted_item_ids
  Given a ContextSources entry referencing item ids {17, 42, 88} where item 42 has since been deleted
  When KnowledgeService::explain runs
  Then it returns items 17 and 88 only, paired with their original weights, in the original order

## Out of Scope

- Auto-calling `commit_outcome` based on `explain` results.
- Recording context sources for the `extractor` path (which does its
  own embedding search) — only the user-visible `MemoryTool::search`
  path emits in this PR. Extractor-time recording is a separate
  follow-up; the failure mode there is different (no user-facing
  turn to attribute back to).
- Surfacing `explain` over HTTP / MCP / UI. Internal Rust API only.
- Adding decay-over-time to confidence.
- A backward-fill of `ContextSources` for historical turns.
- Changing how `default_tape_context` reconstructs LLM messages —
  `ContextSources` is non-conversational like `Anchor` / `Note` /
  `Summary` and is skipped by the existing `_ => {}` arm in
  `crates/kernel/src/memory/context.rs:46..63`.
