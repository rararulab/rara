# web/src/pages — Agent Guidelines

## Purpose

React page-level components that mount pi-web-ui's Lit web components
(`pi-chat-panel`, etc.) and bridge them to rara's storage and streaming layers.

## Architecture

- `PiChat.tsx` — primary chat page; mounts `<pi-chat-panel>`, registers a custom
  message renderer that adds the trace button when an assistant message is
  flagged as the final one of its turn.
- `pi-chat-messages.ts` — pure conversion from rara's `ChatMessageData` shape
  (see `@/api/types`) into pi-agent-core `Message` objects. Owns
  `assistantSeqByRef` (a `WeakMap<AssistantMessage, number>`),
  `toolResultByCallId` (a `Map<string, ToolResultMessage>` populated as a
  side-channel so tool results are NOT emitted as standalone bubbles — see
  #1718), `messagesForArtifactReconstruction` (re-weaves those results back
  in for `ArtifactsPanel.reconstructFromMessages`), and the
  `finalAssistantIndices` helper that computes which assistant per turn is
  the "final" one.
- `pi-chat-messages.test.ts` — unit tests for the conversion + final-assistant
  gating logic.
- Other pages (`Agents.tsx`, `Skills.tsx`, `McpServers.tsx`, etc.) follow the
  same Lit-component-bridge pattern at smaller scale.

Data flow for trace buttons:

```
ChatMessageData[]                     (server)
      │
      ▼
toAgentMessages()                     (writes assistantSeqByRef)
      │
      ▼
AssistantMessage[]  ──────────────►   pi-web-ui renderer
      │                                       │
      └── same WeakMap import ────────────────┘
                                              │
                                              ▼
                                  seq !== undefined ⇒ show trace button
```

## Critical Invariants

- **WeakMap singleton via ES module identity.** `assistantSeqByRef` lives in
  `pi-chat-messages.ts`, is written by `toAgentMessages`, and is read by the
  renderer registered in `PiChat.tsx`. Both sides MUST import from the same
  module path so ES module semantics give them the same `WeakMap` instance. If
  a future contributor copies the `WeakMap` declaration into another module
  (or shadows the import), the renderer will look up keys in a different map
  and silently never show trace buttons.

- **Final-assistant gating only (#1672).** `assistantSeqByRef` registration
  happens ONLY for the final assistant of each turn. The renderer derives
  `showButtons = seq !== undefined`. Do NOT add a second registration site
  without updating the gating logic — every registered ref becomes a visible
  trace button.

- **Object-identity preservation in `toAgentMessages`.** The function must
  preserve the object identity of the `AssistantMessage` references it pushes
  into `result` and registers in the WeakMap. pi-web-ui's renderer receives
  the same references and looks them up by identity; cloning before
  registration silently breaks the lookup.

## What NOT To Do

- Do NOT inline a copy of `assistantSeqByRef` in another module — breaks
  WeakMap identity, silently disables trace buttons.
- Do NOT register intermediate (tool-call-only) assistant messages in
  `assistantSeqByRef` — defeats #1672. If you need per-iteration trace access,
  add a different mechanism (e.g. a separate keyed map), do not expand the
  buttons gate.
- Do NOT clone or spread (`{ ...msg }`) the `AssistantMessage` between
  registration and the renderer — WeakMap keys are by identity, not by value.
- Do NOT re-introduce standalone `ToolResultMessage` entries in
  `toAgentMessages`' output list. Pi-web-ui's `<message-list>` renders one
  DOM row per message object; under rara's avatar CSS that means one bare
  avatar per tool result. Keep results in `toolResultByCallId` and let the
  custom assistant renderer inline them via `toolResultsById` (#1718).
- Do NOT move the `assistantSeqByRef` declaration to a barrel/re-export file
  without keeping the canonical instance in `pi-chat-messages.ts`; re-exports
  are fine, redeclarations are not.

## Dependencies

- `@/api/types` — rara backend message shape (`ChatMessageData`,
  `ChatToolCallData`).
- `@mariozechner/pi-ai`, `@mariozechner/pi-agent-core` — message types
  (`AssistantMessage`, `Message`) consumed by pi-web-ui.
- `@mariozechner/pi-web-ui` — Lit components (`pi-chat-panel`, etc.) that
  render the converted messages and call back into the registered renderer.
