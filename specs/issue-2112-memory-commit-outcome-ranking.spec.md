spec: task
name: "issue-2112-memory-commit-outcome-ranking"
inherits: project
tags: []
---

## Intent

Issue 2111 lands the storage primitives (`memory_items.confidence`
defaulting to 1.0; append-only `memory_outcomes` table). This issue
makes them load-bearing:

1. A typed `OutcomeKind` enum the kernel can hand to a service method.
2. A `KnowledgeService::commit_outcome(item_ids, kind, tape_entry_id)`
   that writes one `memory_outcomes` row per item **and** updates the
   parent `memory_items.confidence` in the same transaction, using an
   exponential-moving-average update with α = 0.2.
3. The existing embedding-driven search path in
   `MemoryTool::exec_search` (which goes
   `EmbeddingService::search` → `items::get_items_by_ids` → return)
   gets re-ranked: distance is still the primary signal, but
   confidence breaks ties and significantly dampens items below ~0.4.

Reproducer for the failure mode this fixes:

1. User says "I prefer dark mode" at T0. Extractor writes row A with
   confidence = 1.0.
2. User says "actually I prefer light mode" at T+30d. Extractor writes
   row B with confidence = 1.0.
3. Today, `MemoryTool` action `search` with query "user UI preference"
   embeds the query, asks usearch for top-K, fetches the matching
   items, and returns them ordered by *insertion order from
   `get_items_by_ids`* — not even by distance. The agent sees both
   rows on equal footing.
4. After this PR: an explicit
   `commit_outcome(&[A], OutcomeKind::Contradicted, …)` drops row A's
   confidence to ~0.8 the first time and continues to decay as further
   contradictions accumulate. The same search now returns B before A,
   because the ranking is `score = -distance + λ · confidence` with
   λ chosen so that a confidence delta of 0.5 dominates a typical
   distance tie.

The α and λ values are mechanism-tuning constants (no operator-relevant
right answer), so they live as `const` next to the mechanism per
project spec — not as YAML.

Why EMA at α = 0.2:

- Five outcomes of the same kind move confidence by ~67 % toward the
  bound, which matches the "you have to be wrong a handful of times
  before rara stops trusting you" intuition we want.
- Bounded by construction in `[0, 1]` — `c' = c + α(1 − c)` from any
  starting point in `[0, 1]` lands in `[c, 1]`; the failure update
  `c' = c − αc` lands in `[0, c]`. No clamping required.
- Symmetric: a confirmed → contradicted ping-pong returns confidence
  to approximately its starting value (EMA is memoryful but not
  hysteretic).
- Cheap: one multiply-add per outcome, no history scan.

Why the same transaction:

- The pair `(write memory_outcomes row, update memory_items.confidence)`
  is the unit of truth. If only one half lands, `explain()` (issue
  2113) starts lying — either it sees an outcome with no corresponding
  confidence change, or it sees a confidence change with no audit
  row. Both states are silently corrupting. Diesel's
  `transaction()` is the right boundary; the writer pool already
  serializes writes.

Goal alignment: signals 2 ("the user stops asking"), 4 ("every
decision is inspectable" — the audit row is the trace), 5 ("memory
survives time"). Crosses no `NOT` line.

Prior art reviewed (raw):

- `gh issue list --search "commit_outcome"` — 0 issues.
- `gh pr list --search "OutcomeKind ranking"` — 0 PRs.
- `git log --grep=commit_outcome` — 0 commits. Greenfield.
- `rg -n "OutcomeKind" crates/` — 0 matches in the memory subtree.
  (`StepOutcome` exists in the kernel for tool-call lifecycle but is
  unrelated; we use a different name to avoid conflation.)
- Existing search path verified against
  `crates/kernel/src/memory/knowledge/tool.rs:100..132` —
  `exec_search` embeds query, calls `EmbeddingService::search`,
  fetches via `items::get_items_by_ids`, filters by username, returns
  in arrival order. No existing ranking layer; the re-rank is
  net-new code, not a replacement.
- `KnowledgeService` in
  `crates/kernel/src/memory/knowledge/service.rs` currently bundles
  `pools`, `embedding_svc`, `config`. Adding `commit_outcome` here is
  the natural seam — pools for the transaction, no other dep needed.

## Decisions

- **Depends on issue 2111** merged into `main` — the column and
  table must exist before this PR's code references them.
- **New file `crates/kernel/src/memory/knowledge/outcome.rs`** —
  hosts the `OutcomeKind` enum, its serialization, and the
  pure `apply_ema` helper. Keeping the enum + math in its own file
  makes it trivial to unit-test the EMA without touching the DB.
- **`OutcomeKind` enum**:
  ```
  pub enum OutcomeKind {
      Success,
      Failure,
      Confirmed,
      Contradicted,
  }
  ```
  `as_str` returns `"success" | "failure" | "confirmed" |
  "contradicted"`. The polarity table:
  - `Success`, `Confirmed` → positive update.
  - `Failure`, `Contradicted` → negative update.
  Why both pairs (and not just one): the source of an outcome carries
  meaning beyond the sign. `Success` typically comes from automated
  tool-result correlation; `Confirmed` typically comes from explicit
  user feedback. The audit row preserves this distinction even
  though the EMA math collapses to two cases.
- **EMA constants** (mechanism, `const`, NOT YAML):
  - `pub const CONFIDENCE_ALPHA: f32 = 0.2;`
  - Update: positive → `c' = c + α(1 − c)`; negative → `c' = c − αc`.
- **`KnowledgeService::commit_outcome`** signature:
  ```
  pub async fn commit_outcome(
      &self,
      item_ids: &[i64],
      kind: OutcomeKind,
      tape_entry_id: Option<i64>,
  ) -> Result<()>;
  ```
  Semantics: opens one writer transaction; for each `item_id` it
  (a) inserts a `memory_outcomes` row, (b) reads the current
  confidence, (c) computes the new value via `apply_ema`, (d) writes
  it back. Empty `item_ids` slice is a no-op (Ok). Unknown
  `item_ids` (rows that do not exist) are silently skipped at the
  inner loop level — the outer call still returns Ok. Rationale:
  callers (notably future tool-result handlers) should not need to
  worry about a memory item having been deleted between embed-time
  and feedback-time; a missing row means "no confidence to update",
  not a programmer error. This decision is deliberate; a follow-up
  could surface a count of skipped ids if needed.
- **Ranking constant**:
  `pub const CONFIDENCE_RANK_WEIGHT: f32 = 0.5;`
  Combined score for the re-rank: `score = -distance + λ · confidence`.
  Items are sorted by `score` descending. Rationale for λ = 0.5:
  usearch cosine distances are in `[0, 2]`, but practical retrieval
  distances cluster in `[0, 0.5]`. A confidence swing of 1.0 (fully
  trusted vs fully decayed) shifts the score by 0.5 — enough to beat
  a typical distance tiebreak (~0.1) but not enough to drag in
  semantically irrelevant matches that already got into the top-K.
  Items with `confidence < 0.4` are still surfaced if they are the
  closest matches, just demoted below higher-confidence neighbors.
- **Re-rank location**: inside `MemoryTool::exec_search` in
  `crates/kernel/src/memory/knowledge/tool.rs`. The existing call
  chain is `embed → search → get_items_by_ids → filter by username
  → JSON-serialize`. The re-rank inserts between
  `get_items_by_ids` and `filter by username`: join the
  `(id, distance)` tuples from `EmbeddingService::search` with the
  loaded `MemoryItem`s by id, compute `score`, sort, then proceed.
  No public API change to the tool action.
- **Do NOT expose `commit_outcome` to the LLM via the existing
  `MemoryTool` actions in this PR.** The tool currently exposes
  `search | categories | read_category`. Adding a `commit_outcome`
  action means the model can self-attest its own memory's truth,
  which inverts the feedback loop. The internal Rust API is the
  only caller in this PR; user-driven and tool-result-driven
  callers are scoped for a later issue.
- **Schema reuse**: write `memory_outcomes` via a small private
  diesel insert in `outcome.rs`. Do NOT add a `NewMemoryOutcome`
  struct to `items.rs` — that file already has a tight focus on
  `memory_items` rows and crossing the file boundary is the
  cleanest cut between issues 2111 and 2112.
- **No `Default` impl** for `OutcomeKind`. There is no sensible
  default outcome; every call site must pick one explicitly.
- **`apply_ema` is pure** (`fn apply_ema(current: f32, kind:
  OutcomeKind) -> f32`) so the EMA math has a unit-test foothold
  that does not need diesel or a runtime.

## Boundaries

### Allowed Changes
- crates/kernel/src/memory/knowledge/outcome.rs
- **/crates/kernel/src/memory/knowledge/outcome.rs
- crates/kernel/src/memory/knowledge/mod.rs
- **/crates/kernel/src/memory/knowledge/mod.rs
- crates/kernel/src/memory/knowledge/service.rs
- **/crates/kernel/src/memory/knowledge/service.rs
- crates/kernel/src/memory/knowledge/tool.rs
- **/crates/kernel/src/memory/knowledge/tool.rs
- specs/issue-2112-memory-commit-outcome-ranking.spec.md
- **/specs/issue-2112-memory-commit-outcome-ranking.spec.md

### Forbidden
- crates/rara-model/**
- crates/kernel/src/memory/knowledge/items.rs
- crates/kernel/src/memory/knowledge/extractor.rs
- crates/kernel/src/memory/knowledge/embedding.rs
- crates/kernel/src/memory/knowledge/config.rs
- crates/kernel/src/memory/knowledge/categories.rs
- crates/kernel/src/memory/knowledge/manifest.rs
- crates/kernel/src/memory/context.rs
- crates/kernel/src/memory/store.rs
- config.example.yaml
- .github/workflows/**
- docs/**
- web/**

## Completion Criteria

Scenario: apply_ema converges toward 1.0 under positive outcomes
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::outcome::tests::ema_positive_monotone_toward_one
  Given a starting confidence c in (0, 1) and OutcomeKind::Confirmed
  When apply_ema runs five times in succession
  Then the result is strictly increasing each step and stays within [c, 1]

Scenario: apply_ema decays toward 0 under negative outcomes
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::outcome::tests::ema_negative_monotone_toward_zero
  Given a starting confidence c in (0, 1) and OutcomeKind::Contradicted
  When apply_ema runs five times in succession
  Then the result is strictly decreasing each step and stays within [0, c]

Scenario: commit_outcome writes audit row and updates confidence atomically
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::outcome::tests::commit_outcome_writes_row_and_updates_confidence
  Given a memory_items row with confidence 1.0
  When KnowledgeService::commit_outcome is called with OutcomeKind::Failure and tape_entry_id Some(42)
  Then a memory_outcomes row exists with outcome_kind = "failure" and tape_entry_id = 42, and the parent row's confidence equals 0.8

Scenario: commit_outcome skips unknown item ids without erroring
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::outcome::tests::commit_outcome_skips_unknown_ids
  Given an item_ids slice that contains one valid id and one id with no matching row
  When KnowledgeService::commit_outcome is called
  Then it returns Ok and only the valid row's confidence changes

Scenario: search re-ranks higher-confidence items above lower-confidence ones
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::tool::tests::search_prefers_high_confidence_on_ties
  Given two memory items with near-identical embeddings, where item A has confidence 0.95 and item B has confidence 0.3
  When MemoryTool::exec_search runs against a query that matches both
  Then item A appears before item B in the returned items array

## Out of Scope

- Adding the schema (column + table) — that is issue 2111's job.
- Exposing `commit_outcome` to the LLM as a `MemoryTool` action.
  Deferred deliberately; we want the loop running internally first.
- Wiring tool-result success/failure to auto-call `commit_outcome`.
  The tool-result correlation layer is a separate change.
- Recording which items contributed to a given turn (the
  `tape_entry_id` parameter is there, but populating it from real
  LLM turns is issue 2113's job).
- Boosting/decay decay-over-time on confidence (a separate signal
  from outcome-driven feedback).
- UI surfacing of confidence values.
- Changing `MemoryItem`'s JSON shape sent to the LLM — the existing
  search-action response keys stay identical.
