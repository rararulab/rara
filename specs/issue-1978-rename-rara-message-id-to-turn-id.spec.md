spec: task
name: "issue-1978-rename-rara-message-id-to-turn-id"
inherits: project
tags: ["kernel", "channels", "ui", "core"]
---

## Intent

The field `rara_message_id` is named after a single message, but it
semantically identifies an entire turn. One inbound user message produces
exactly one `rara_message_id`, and that same id is then attached as
metadata to every tape entry generated during the turn — the user message
entry, the assistant text entry, the reasoning entry, every tool_call
entry, and every tool_result entry. A modest single-tool turn already
fans the same id out to 5 tape entries; a multi-tool turn easily reaches
9+.

Concrete evidence (verified in production data):
- Tape file: `/Users/rara/Library/Application Support/rara/memory/tapes/315bad61f8c34b137221bd6ec086597c__d6e905d9-fd62-41ca-8918-97b37276f534.jsonl`
- The id `27df73bb-9656-4e23-bbb9-e724e37162c7` appears in the
  `metadata.rara_message_id` of TapEntry ids 36, 38, 39, 40, 42, 43, 44,
  46 — nine entries, one id.

This is intentional in the kernel layer (see Prior art below) — the id
is the turn-level correlation handle that powers `entries_by_message_id`,
which the `/debug` Telegram command, `rara debug <id>` CLI, the
`debug_trace` agent tool, and the web `ExecutionTraceModal` all use to
retrieve the full set of entries belonging to one turn. The semantics are
correct. **The name lies about them.** Worse, the user-facing surfaces
amplify the lie:

- Telegram detail panel renders a row literally labelled "Message ID".
- `web/src/components/chat/ExecutionTraceModal.tsx` displays it as the
  trace's message id.
- `web/src/api/kernel-types.ts` exposes the field as `rara_message_id`
  to every frontend consumer.
- The `debug_trace` agent tool description says "search by
  rara_message_id" — so when rara itself reasons about its own debug
  flow, it reads the misleading word.
- The CLI subcommand is `rara debug <message_id>`.

Reproducer for "what bug appears if we don't do this": (1) a user
reports a bug and includes "this is the message id: `27df...`",
copy-pasted from the Telegram detail panel. (2) Whoever debugs (human or
rara via `debug_trace`) opens the trace and gets back nine tape entries
all carrying that id. (3) The natural follow-up question — "which one
is the actual user message? which one is the assistant reply you saw?"
— has no clean answer because the user assumed one ID = one message.
The debug round-trip burns extra context purely on naming-induced
confusion. (4) Tooling that takes a `rara_message_id` argument (CLI,
agent tool, telegram `/debug`) returns a turn-shaped result, not a
message-shaped result, but the parameter name promises the latter. The
mismatch is invisible until the user is already confused. The bug is
not "the system computes the wrong thing" — it is "the system's
self-description teaches every consumer the wrong mental model."

Goal alignment: this advances `goal.md` signal 4 ("every action is
inspectable. Each decision can be pulled from the eval interface as a
raw trace, score, and replayable record. No 'I don't know why it did
that.'"). The primary debug handle on the primary inspection surface
must teach the right mental model. Mislabelling the most-used
correlation id directly degrades inspectability across Telegram, web,
CLI, and the agent's own self-debug path. Does not cross any "What rara
is NOT" line.

Hermes positioning: not applicable. This is internal naming hygiene on
rara's debug substrate, no analog in Hermes Agent's public surface.

Prior art search summary (the wall this spec must clear):

- `gh issue list --search "rara_message_id" --state all` returned:
  - issue 335 (closed) "feat(kernel,telegram): rara_message_id end-to-end
    tracing and debug_trace tool" — **the original design**. Spec
    explicitly states: "InboundMessage.id ... is the entire turn's
    rara_message_id, all the way through agent loop -> TurnTrace ->
    TurnMetrics -> channel adapter." So the per-turn semantics are
    intentional and load-bearing; only the name is wrong.
  - issue 1127 "/debug command for Telegram message context retrieval" —
    consumer of the field; depends on the same per-turn fan-out.
  - issue 548 "enrich tape entry metadata with latency, model" —
    extended `LlmEntryMetadata` alongside `rara_message_id`.
  - issue 1613 "own ExecutionTrace construction + persistence" —
    refactored storage of the trace but kept the field name.
- `gh pr list --search "rara_message_id" --state all` returned PRs 337,
  339, 1136, 1138, 1156, 1614 — every PR that has touched this field
  reinforced its current name. None proposed a rename. None challenged
  the naming.
- `git log --grep "rename.*message_id|turn_id"` since 365 days returned
  no hits. Nobody has attempted this rename before.
- `git log --grep "trigger_message_id"` returned commit 002c81e1 "track
  trigger_message_id on background tasks" — there is one adjacent
  field, `trigger_message_id`, on background tasks that is genuinely
  per-message (it points at the inbound that *triggered* the
  background task). That field's name is correct as-is and is out of
  scope.
- `rg "rara_message_id"` returned 75 references across 21 files
  (kernel, channels, app, cmd, extensions, web). Comprehensive list
  captured in Allowed Changes below.
- `rg "rara_turn_id"` returned 0 hits. Name is free.

No prior decision is being reversed. Issue 335 chose
"`InboundMessage.id` is the turn id" — that decision stands. This spec
only renames the field to match what 335 already declared it does.

## Decisions

### Rename, do not redesign

The kernel-level semantics ("one id per turn, attached to every entry of
that turn, used to retrieve the turn's full execution context") are
correct and stay. This spec changes the *name* on every surface to
`rara_turn_id`. No new field is introduced; no per-message id is
invented; the existing `TapEntry.id: u64` remains the per-entry handle
for anyone who actually needs to point at one entry inside a turn.

The single test that nails this lane: a `Test:` selector binds to the
existing `entries_by_message_id` test surface (renamed to
`entries_by_turn_id`); the function returns the same nine entries it
returned before, but the field on `LlmEntryMetadata` and
`ToolResultMetadata` is now `rara_turn_id`. Fail before, pass after.

### Tape JSONL is on-disk data; backward compatibility is mandatory

Existing tape files in `~/Library/Application Support/rara/memory/tapes/`
have `"rara_message_id"` literal keys baked into their JSON metadata.
They cannot be retroactively rewritten — they are append-only artefacts
of past sessions. The deserializer must accept both keys; the serializer
writes only the new key.

Concretely: `LlmEntryMetadata` and `ToolResultMetadata` use a
`#[serde(alias = "rara_message_id")] pub rara_turn_id: String` for the
field. Serde aliases are only used during deserialization, so old tapes
load and new tapes write `rara_turn_id`. The JSON-pointer lookups in
`memory/service.rs::entries_by_turn_id` (and the duplicate logic in
`debug_trace.rs`, `debug.rs`, `chat/service.rs`) must check both keys —
encapsulated in a single helper, e.g.
`fn read_turn_id(metadata: &Value) -> Option<&str>` that tries
`rara_turn_id` first, falls back to `rara_message_id`. Producer-side
code only ever writes the new key.

### User-facing surface labels follow

The naming lie is most damaging at the surfaces the user reads. The
rename includes:

- Telegram detail panel: "Message ID" label becomes "Turn ID"
  (`crates/channels/src/telegram/adapter.rs` near line 857).
- Web `ExecutionTraceModal.tsx`: the displayed label and any prop names
  follow.
- CLI `rara debug <message_id>` becomes `rara debug <turn_id>`. The
  positional argument keeps the same shape (a UUID); only the label and
  help text change. No deprecated alias for the CLI flag is introduced
  — the CLI takes a positional, not a named flag, so there is nothing
  to alias.
- `debug_trace` agent tool: parameter name and tool description rename.
  The tool's input JSON schema property changes from `rara_message_id`
  to `rara_turn_id`. **Backward-compat for the LLM:** the tool's input
  deserializer accepts both keys (same alias trick) so an old prompt
  cache referencing the old name does not produce a hard failure. New
  tool description and schema only mention the new name.

### TurnTrace and TurnMetrics field names rename together

`TurnTrace.rara_message_id` and `StreamEvent::TurnMetrics.rara_message_id`
both rename. The frontend type `web/src/api/kernel-types.ts` follows.
There is no in-flight network compatibility story to preserve — the
backend and frontend ship as one binary on the remote in this repo's
deployment model, and stream events are not persisted.

### Why not "add docs and AGENT.md instead"

Documentation cannot fix a misleading public string. The user reads
"Message ID" in the UI, copies that ID, says "this message ID";
the engineer or the agent looks at a parameter named `rara_message_id`
and reasons about it as a message id. AGENT.md and a `///` doc comment
are read by agents writing kernel code, not by users reading the UI or
LLMs invoking `debug_trace`. Doc-only fixes leave the failure mode in
place at every surface where it actually triggers.

### Why not "split into per-message + per-turn ids"

`TapEntry.id: u64` (defined in `crates/kernel/src/memory/mod.rs:340`)
already serves as the per-entry handle. The reproducer's nine-entry
turn was identified by ids "36, 38, 39, 40, 42, 43, 44, 46" — that
numbering is `TapEntry.id`. Adding a second uuid-shaped per-message id
to every tape entry would be net-new feature work with no current
consumer; conflating it with this rename would balloon the diff and
muddy the BDD scenarios. If a per-entry user-facing UUID is needed
later, it lands as a separate issue.

## Boundaries

### Allowed Changes

- **/crates/kernel/src/memory/mod.rs
- **/crates/kernel/src/memory/service.rs
- **/crates/kernel/src/agent/mod.rs
- **/crates/kernel/src/kernel.rs
- **/crates/kernel/src/io.rs
- **/crates/kernel/src/trace/builder.rs
- **/crates/kernel/src/trace/mod.rs
- **/crates/kernel/src/plan.rs
- **/crates/kernel/src/debug.rs
- **/crates/kernel/src/tool/mod.rs
- **/crates/kernel/src/tool/schedule.rs
- **/crates/kernel/src/tool/background_common.rs
- **/crates/app/src/tools/debug_trace.rs
- **/crates/app/src/tools/send_file.rs
- **/crates/channels/src/telegram/adapter.rs
- **/crates/channels/src/telegram/commands/debug.rs
- **/crates/channels/src/web.rs
- **/crates/cmd/src/debug.rs
- **/crates/cmd/src/chat/mod.rs
- **/crates/extensions/backend-admin/src/chat/service.rs
- **/web/src/api/kernel-types.ts
- **/web/src/components/chat/ExecutionTraceModal.tsx
- **/crates/kernel/tests/tape_metadata_back_compat.rs
- **/specs/issue-1978-rename-rara-message-id-to-turn-id.spec.md

Producer-side rename (struct field + serializer):

- `crates/kernel/src/memory/mod.rs` — rename
  `LlmEntryMetadata.rara_message_id` and
  `ToolResultMetadata.rara_message_id` to `rara_turn_id`; add
  `#[serde(alias = "rara_message_id")]` on each.
- `crates/kernel/src/agent/mod.rs` — rename `TurnTrace.rara_message_id`
  to `rara_turn_id`; rename all local `let rara_message_id = ...`
  bindings; rename function parameters and field initializers.
- `crates/kernel/src/kernel.rs` — rename the metadata-emission sites
  (lines around 1702 and 2324 that build
  `serde_json::json!({"rara_message_id": ...})` to write
  `"rara_turn_id"` instead) and rename `let rara_message_id = ...`
  bindings around the turn dispatch.
- `crates/kernel/src/io.rs` — rename `StreamEvent::TurnMetrics`
  field around line 1004.
- `crates/kernel/src/trace/builder.rs`,
  `crates/kernel/src/trace/mod.rs` — rename through the trace
  builder.
- `crates/kernel/src/plan.rs` — rename usages.
- `crates/kernel/src/debug.rs` — rename uses, including the doc
  comments at the top of the module.
- `crates/kernel/src/tool/mod.rs`,
  `crates/kernel/src/tool/schedule.rs`,
  `crates/kernel/src/tool/background_common.rs` — rename
  call sites.
- `crates/kernel/src/memory/service.rs` —
  rename `entries_by_message_id` to `entries_by_turn_id`; the
  inner JSON-pointer lookup uses a helper that reads `rara_turn_id`
  first, falls back to `rara_message_id`.

Consumer rename:

- `crates/app/src/tools/debug_trace.rs` — rename the agent-tool
  parameter to `rara_turn_id`, update tool description and JSON schema,
  add input deserialization alias for `rara_message_id`.
- `crates/app/src/tools/send_file.rs` — rename the test/synthetic
  field follower (line 233 area).
- `crates/channels/src/telegram/adapter.rs` — rename the
  `rara_message_id` references; update the user-facing label
  ("Message ID" becomes "Turn ID") around line 857; rename
  `let rara_message_id = ...` around line 2430.
- `crates/channels/src/web.rs` — rename the destructure
  `rara_message_id: _`.
- `crates/cmd/src/debug.rs` — rename CLI argument, help text, and
  doc comments; the CLI takes a positional UUID so only labels change.
- `crates/cmd/src/chat/mod.rs` — rename the destructure.
- `crates/extensions/backend-admin/src/chat/service.rs` — rename
  the `let rara_message_id = ...`, the metadata lookup (use the
  same helper that handles both keys), and the doc comments around
  lines 731 to 810.

Frontend rename:

- `web/src/api/kernel-types.ts` — rename the two field declarations
  (lines 111 and 481) from `rara_message_id: string` to
  `rara_turn_id: string`.
- `web/src/components/chat/ExecutionTraceModal.tsx` — rename the
  property access on line 156 and update the displayed label from
  "Message ID" to "Turn ID".

Tests:

- `crates/kernel/tests/tape_metadata_back_compat.rs` — new test, see
  Completion Criteria.
- Existing tests that reference `rara_message_id` follow the rename.
  No new test names are introduced beyond the back-compat test;
  scenario selectors below bind to it and to the `entries_by_turn_id`
  surface.

Doc/comment updates:

- All `///` doc comments and `//!` module-level docs in the files
  above that mention "rara_message_id" follow the rename. The
  per-turn semantics (one id per inbound message, attached to every
  entry of the resulting turn) become the explicit doc text rather
  than implicit knowledge.

### Forbidden

- Do NOT introduce a per-entry user-facing UUID alongside `rara_turn_id`.
  `TapEntry.id: u64` already covers per-entry addressing. Inventing a
  second handle is the option C alternative, explicitly rejected.
- Do NOT migrate or rewrite existing tape JSONL files on disk. They
  remain append-only history; the loader's `serde(alias)` is the entire
  back-compat surface.
- Do NOT remove the `serde(alias = "rara_message_id")` or the JSON-key
  fallback in the metadata-lookup helper. They are the contract that
  keeps every existing tape file readable. Removing them is a future
  decision behind its own issue.
- Do NOT rename `trigger_message_id` (on background tasks). That field
  genuinely is per-message and its name is correct.
- Do NOT change `OutboundEnvelope.id` or `OutboundEnvelope.in_reply_to`.
  Those are unrelated to this rename.
- Do NOT introduce a deprecation warning emitted at runtime when an old
  tape with `rara_message_id` is loaded. Silent acceptance is the
  desired UX — it is not an error condition.
- Do NOT add a YAML config knob to control whether the writer emits
  the old name. Per the mechanism-vs-config rule, this is not config
  surface.
- Do NOT mark any new test `#[ignore]`.
- Do NOT introduce a separate Rust type alias `pub type TurnId =
  String` in this PR. The existing `crate::io::MessageId` newtype stays
  on the kernel side as the typed handle (since it really is the
  inbound message id that becomes the turn id); only the *field name*
  on metadata structs and on TurnTrace/TurnMetrics renames. A typed
  `TurnId` newtype is a future refactor beyond this issue's scope.

## Completion Criteria

Scenario: Tape metadata serializes the new turn-id key
  Test:
    Package: rara-kernel
    Filter: tape_metadata_back_compat::serializes_rara_turn_id
  Given a fresh LlmEntryMetadata constructed with rara_turn_id "abc-123"
  When the metadata is serialized to JSON via serde_json::to_value
  Then the resulting JSON object contains key "rara_turn_id" with value "abc-123" and contains no key named "rara_message_id"

Scenario: Tape metadata deserializes legacy rara_message_id key
  Test:
    Package: rara-kernel
    Filter: tape_metadata_back_compat::accepts_legacy_rara_message_id
  Given a JSON object containing the key "rara_message_id" with value "legacy-id-xyz" and the other required LlmEntryMetadata fields
  When the JSON is deserialized into LlmEntryMetadata via serde_json::from_value
  Then deserialization succeeds and the resulting metadata's rara_turn_id field equals "legacy-id-xyz"

Scenario: entries_by_turn_id returns all entries of a turn for both legacy and new metadata
  Test:
    Package: rara-kernel
    Filter: tape_metadata_back_compat::entries_by_turn_id_back_compat
  Given a tape containing two entries whose metadata uses the legacy key "rara_message_id" set to "turn-A" and one entry whose metadata uses the new key "rara_turn_id" also set to "turn-A"
  When TapeService::entries_by_turn_id is invoked with "turn-A"
  Then exactly three entries are returned

Scenario: debug_trace agent tool accepts both old and new parameter names
  Test:
    Package: rara-app
    Filter: debug_trace::accepts_legacy_rara_message_id_param
  Given a debug_trace tool input JSON object that contains the legacy key "rara_message_id" set to "trace-1"
  When the input is deserialized into the tool's input struct
  Then deserialization succeeds and the parsed rara_turn_id equals "trace-1"

## Out of Scope

- Introducing a typed `TurnId` newtype distinct from `MessageId` on the
  Rust side. The current `MessageId` newtype keeps semantic accuracy at
  the IO layer ("this is the inbound message id") and the rename only
  needs to reach as far as the *field names* on metadata and trace
  structs.
- Adding a per-entry user-facing UUID handle. `TapEntry.id: u64` is the
  existing primitive and is sufficient.
- Renaming `trigger_message_id` on background tasks. That field is
  genuinely per-message.
- Migrating or rewriting on-disk tape JSONL files. Back-compat lives in
  the deserializer.
- Removing the `serde(alias = "rara_message_id")` after some grace
  period. That is a future decision behind its own issue, after
  evidence shows no recent tape file still uses the old key.
- Any change to `OutboundEnvelope.id`, `OutboundEnvelope.in_reply_to`,
  or the inbound/outbound id semantics generally.
- Telemetry / OTel attribute renames (e.g. if a span attribute is
  named `rara.message_id`). If any such attribute exists, it follows
  in a separate issue scoped to telemetry semconv (issue 1856 line of
  work).
