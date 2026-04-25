# web/src/pages — Agent Guidelines

## Purpose

React page-level components. The chat page (`PiChat.tsx`) renders the
ported ai-elements `Conversation` + `Message` primitives against rara's
WebSocket chat stream.

## Architecture

- `PiChat.tsx` — primary chat page mounted at `/`. Owns session selection,
  history fetch, optimistic user-message append, WebSocket lifecycle, and
  the cascade / execution-trace modals. Streams events through
  `applyRaraEvent` (see `@/components/chat/rara-to-uimessage`) which folds
  WebSocket frames into a `RaraUIMessage[]`.
- `Login.tsx`, `Docs.tsx`, `KernelTop.tsx`, `Dock.tsx`, `Subscriptions.tsx`
  — admin / surface pages with their own routing under `DashboardLayout`.

Data flow for the chat page:

```
WebSocket frames (PublicWebEvent)
        │
        ▼
applyRaraEvent ────► RaraUIMessage[]
                            │
                            ▼
                    <Conversation> + <Message> + <Tool>
                            │
                            ▼
              per-turn metadata.seq drives trace / cascade triggers
```

## Critical Invariants

- **`activeSessionRef` race guard.** A→B→A session switching can resolve
  history fetches in the wrong order. `selectSession` and
  `reloadActiveMessages` MUST compare `activeSessionRef.current` against
  the captured key before applying state, otherwise stale history clobbers
  the active session.
- **Close the previous WebSocket before mutating state on session
  switch.** Failing to do so lets in-flight assistant frames from the old
  session land on the new session via `setMessages` and bleed across.
- **Trace / cascade triggers only render when `metadata.seq` is defined.**
  rara only assigns `seq` once a turn is persisted; in-flight assistant
  turns have `seq === undefined`. Mounting the trigger row earlier would
  make the buttons 404.

## What NOT To Do

- Do NOT bypass `applyRaraEvent` to mutate `messages` directly — the
  adapter centralises the per-frame state machine (text accumulation,
  tool-call lifecycle, error injection).
- Do NOT seed the composer text on a suggestion click — the empty-state
  flow goes straight through `sendMessage(prompt)` so the user does not
  have to press enter.

## Dependencies

- `@/adapters/rara-stream` — `PublicWebEvent` discriminated union and
  `buildWsUrl(sessionKey)` helper.
- `@/components/chat/rara-to-uimessage` — `applyRaraEvent` reducer and
  `historyToUIMessages` history bootstrapping.
- `@/components/chat/ai-elements/*` — ported `Conversation`, `Message`,
  `Tool`, `PromptInput` primitives.
- `@/api/client` — REST calls for sessions, history, trace, cascade.
