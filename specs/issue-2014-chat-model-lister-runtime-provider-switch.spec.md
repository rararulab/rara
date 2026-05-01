spec: task
name: "issue-2014-chat-model-lister-runtime-provider-switch"
inherits: project
tags: []
---

## Intent

Switching `llm.default_provider` in Settings does not take effect until the
server is restarted. Concretely, the `GET /api/v1/chat/models` endpoint
keeps returning models from whichever provider was active at boot, and the
chat turn path keeps routing through the boot-time driver. The user
reports this as "model picker 切了 provider 还是 minimax，是不是写死了？".

Reproducer (the bug appears today):

1. Start `rara server` with `~/.config/rara/config.yaml` set so
   `llm.default_provider: minimax`. Both `minimax` and `openrouter`
   providers are configured with valid keys.
2. Confirm `GET http://10.0.0.183:25555/api/v1/chat/models` returns
   minimax model ids (e.g. `MiniMax-M2`).
3. PATCH the setting via UI or `PUT /api/v1/settings`:
   `llm.default_provider = openrouter`.
4. `GET /api/v1/chat/models` again — still minimax ids, even after the
   `ModelCatalog` 5-minute TTL expires (the `LlmModelListerRef` itself is
   bound to the minimax driver, so a fresh fetch hits the same backend).
5. Send a chat turn — driver resolution at the kernel/turn level is also
   stuck on the boot-time `default_driver`, since nothing re-resolves on
   setting change.
6. `pkill rara && just run` recovers; this confirms the issue is purely
   runtime mutability, not data corruption.

Root cause (verified against the tree at d5eb2e26):

- `crates/app/src/boot.rs:275-307` reads `llm.default_provider` once at
  boot and clones the chosen `OpenAiDriver` into both `model_lister:
  LlmModelListerRef` and `embedder: LlmEmbedderRef`. These are plain
  `Arc`s without indirection; after boot, no code path swaps them when
  the setting changes.
- `crates/extensions/backend-admin/src/chat/model_catalog.rs:33,162-227`
  caches the result for `CACHE_TTL = 5 minutes` with no invalidation
  hook. Even if `model_lister` were re-bound, the cache would mask the
  switch for up to 5 minutes.
- `crates/kernel/src/llm/registry.rs:120-140` already holds **all**
  registered drivers keyed by name and a mutable `default_driver:
  String` — the right datum is already there; nothing reads it
  dynamically for the model-lister role.
- `web/src/hooks/use-chat-models.ts:33` adds a frontend react-query
  `staleTime: 5min` that keeps the stale list visible even after the
  backend would return fresh data, but the frontend layer is downstream
  of the backend bug — fixing it without fixing the backend would still
  leave `curl /api/v1/chat/models` wrong.

Prior-art search (run per spec-author rules):

- `gh issue list "model picker"` / `"provider switch"` / `"minimax provider"`:
  several adjacent issues — #1276 (provider switching UI), #1554
  (rara-native model dialog), #1670 (background-agent runtime
  mutability), #1670 itself was shipped in PR #1671. None of them
  address the chat-path `model_lister` Arc captured at boot. **#1670 is
  the closest relative**: its body explicitly says "Agent driver/model
  choices should be mutable via settings.db without a restart, so
  operators can switch providers live (e.g. during provider outages or
  A/B testing)." That issue scoped the fix to background agents
  (`knowledge_extractor`, `title_gen`); the user-facing chat path was
  out of scope. This issue is the natural follow-on, not a contradiction.
- `git log --grep=default_provider --since=180.days`: PR #1999 series
  (multi-agent observability), PR #1670/#1671 (agent fallback chain),
  PR #1542 (provider allowlist), PR #1554/#1570 (rara-native dialog and
  clear-override). None re-bind `model_lister` on setting change.
- `rg default_provider` and `rg model_lister`: `boot.rs` is the only
  site that consumes `llm.default_provider` to construct the
  `LlmModelListerRef`. There is no existing settings-watcher / event
  hook for it to subscribe to.

No prior PR has been reverted in this area. The work does not collide
with #1670's resolution chain — it extends the same principle to the
chat-path catalog.

Goal alignment: signal 4 in `goal.md` — "every action is inspectable;
no 'I don't know why it did that'". When a user changes a setting and
the system silently keeps the old behavior for 5 minutes (or forever,
until restart), that is exactly the "I don't know why it did that"
failure mode. Crosses no `NOT` line — this is single-user, single-process,
not feature-parity-with-Hermes (Hermes runs a fixed model; runtime
provider switching is rara-specific because of the Chinese-provider mix).

Hermes-Agent check: Hermes does not expose runtime provider switching as
a user-facing feature (one model per deployment). rara has a real
engineering reason to differ — operators flip between minimax /
openrouter / kimi during outages and A/B tests, and #1670 already
committed to "live switch without restart" as a project value.

## Decisions

- **Source of truth: `DriverRegistry`, not a captured `Arc`.** The boot
  code stops cloning a single `OpenAiDriver` into `model_lister` and
  `embedder`. Instead, the model-lister role and the embedder role both
  resolve through the registry's current `default_driver` at call time.
  The registry already owns every driver by name and has a mutable
  `default_driver: String` field (`crates/kernel/src/llm/registry.rs:66`)
  — we route through it.
- **Mechanism: a thin `LlmModelLister` adapter** (and matching
  `LlmEmbedder` adapter) that holds `DriverRegistryRef` and, on each
  call, looks up the current default driver and delegates. No new trait
  shape, no new config key. The adapter is a `pub struct` next to
  `DriverRegistry` in `crates/kernel/src/llm/`. This keeps the public
  `LlmModelListerRef` / `LlmEmbedderRef` types unchanged so downstream
  callers (`backend-admin/chat/service.rs`, knowledge layer) need zero
  changes.
- **Settings-driven re-resolution.** The registry's `default_driver`
  string must be updated when `llm.default_provider` changes in the
  settings DB. We add a public `set_default_driver(&str)` method on
  `DriverRegistry` and call it from the existing settings-write path
  (`PATCH /api/v1/settings` handler in `backend-admin`). No polling,
  no event bus subscription — direct call from the write handler is
  the smallest mechanism that matches #1670's pattern for background
  agents (which also takes effect on the next read after a settings
  write, no events required).
- **`ModelCatalog` cache invalidation.** Add a public
  `ModelCatalog::invalidate()` that drops the cached entry, called
  from the same settings-write path when `llm.default_provider`
  changes. The 5-minute TTL stays for the no-change case (it exists
  to spare the upstream provider's catalog endpoint from per-request
  hits, which is still valid). `CACHE_TTL` remains a `const` per the
  project spec's "mechanism constants are not config" rule.
- **Frontend cache invalidation.** `web/src/hooks/use-chat-models.ts`
  must invalidate its react-query cache when the user mutates the
  `llm.default_provider` setting. The existing settings PATCH hook
  already exists; it gains a `queryClient.invalidateQueries(['chat',
  'models'])` call. No new state machine.
- **No Rust defaults.** Boot still reads `llm.default_provider` from
  YAML / settings DB exactly as today; the registry's `default_driver`
  is initialized from that value. No fallback string is hardcoded as a
  "sensible default" in Rust — anti-pattern per
  `docs/guides/anti-patterns.md`. (The existing
  `unwrap_or_else(|| "openrouter".to_owned())` in `boot.rs:280` is
  pre-existing and out of scope; touching it would expand blast radius
  and is tracked separately if at all.)
- **Embedder: same shape.** The embedder Arc is captured the same way
  at boot and feeds the knowledge layer (`init_knowledge_service`).
  We re-bind it through the registry too, for symmetry and to avoid
  the next "embeddings are stuck on the old provider" report. Tests
  are scoped to the model-lister path (the user-visible bug); the
  embedder change is verified by existing knowledge-layer tests
  continuing to pass.
- **Out of scope: provider hot-reload of credentials.** If the user
  changes `llm.providers.openrouter.api_key` while the server is
  running, this issue does not promise that the next call uses the
  new key. That is a separate issue (the credential resolver is
  per-driver and would need its own re-bind path). Switching between
  *already-configured* providers is the contract here.

## Boundaries

### Allowed Changes
- **/crates/app/src/boot.rs
- **/crates/app/src/lib.rs
- **/crates/kernel/src/llm/registry.rs
- **/crates/kernel/src/llm/mod.rs
- **/crates/kernel/src/llm/runtime_lister.rs
- **/crates/kernel/src/llm/runtime_embedder.rs
- **/crates/extensions/backend-admin/src/chat/model_catalog.rs
- **/crates/extensions/backend-admin/src/chat/service.rs
- **/crates/extensions/backend-admin/src/settings/**
- **/crates/extensions/backend-admin/src/state.rs
- **/crates/extensions/backend-admin/tests/**
- **/web/src/hooks/use-chat-models.ts
- **/web/src/components/settings/SettingsPanel.tsx
- **/specs/issue-2014-chat-model-lister-runtime-provider-switch.spec.md

### Forbidden
- **/crates/kernel/src/llm/openai.rs
- **/crates/kernel/src/llm/driver.rs
- **/crates/kernel/src/llm/catalog.rs
- **/crates/kernel/src/agent/**
- **/crates/kernel/src/memory/**
- **/crates/rara-model/migrations/**
- **/config.example.yaml
- **/web/src/components/topology/TimelineView.tsx
- **/.github/workflows/**

## Acceptance Criteria

Scenario: chat models endpoint reflects new provider after settings change
  Test:
    Package: rara-backend-admin
    Filter: chat::model_catalog::tests::switching_default_provider_returns_new_catalog
  Given a backend-admin chat service is wired through DriverRegistry with two providers registered ("provider_a" with model id "model-a-1", "provider_b" with model id "model-b-1") and default_driver initially "provider_a"
  When the test calls list_models, then sets default_driver to "provider_b" via the registry, invalidates the ModelCatalog cache, and calls list_models again
  Then the second call returns the catalog from "provider_b" (contains "model-b-1") and does not contain "model-a-1"

Scenario: settings PATCH on llm.default_provider updates registry and invalidates catalog
  Test:
    Package: rara-backend-admin
    Filter: settings::tests::patch_default_provider_invalidates_chat_model_cache
  Given the settings router is wired with a DriverRegistry containing "provider_a" and "provider_b" and a ModelCatalog whose cache currently holds "provider_a" entries
  When a PATCH /api/v1/settings request sets llm.default_provider to "provider_b"
  Then the registry's default_driver returns "provider_b" and the next ModelCatalog::list_models call performs a fresh fetch (cache reports empty before fetch)

Scenario: cache invalidation does not regress no-change reads
  Test:
    Package: rara-backend-admin
    Filter: chat::model_catalog::tests::ttl_still_caches_when_provider_unchanged
  Given a ModelCatalog with default TTL and a registry whose default_driver is unchanged across two list_models calls within the TTL window
  When list_models is called twice in succession
  Then the underlying LlmModelLister is called exactly once (cache served the second call)

## Out of Scope

- Hot-reload of provider credentials (api_key, base_url) without restart.
- Surfacing per-provider model lists side-by-side in the UI (the picker
  still shows one provider's catalog at a time — the active one).
- TimelineView vendor-icon hardcoded `'pi'` provider field
  (`web/src/components/topology/TimelineView.tsx`) — separate lane-2
  chore as noted by the user.
- Touching the boot-time `unwrap_or_else(|| "openrouter")` fallback in
  `boot.rs`. Pre-existing, orthogonal.
- Any change to `DriverRegistry::resolve_agent` priority chain — this
  issue does not touch agent resolution, only the model-lister/embedder
  roles.
