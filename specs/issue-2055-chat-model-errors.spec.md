spec: project
name: "issue-2055-chat-model-errors"
---

## Intent

Chat prompt submission must keep session model/provider pins coherent and
must surface backend LLM errors in the Chat UI. A failed provider call should
not look like an assistant that is still thinking.

## Constraints

Production session `a321d66a-2d59-43e1-a199-7faef7bc70cd` had a stale
session override:

- `model = kimi-for-coding`
- `model_provider = minimax`

The Chat model picker listed `kimi-for-coding` from the current model catalog,
then the session WebSocket sent only the model id. The backend updated
`SessionEntry.model` but left `SessionEntry.model_provider` as `minimax`, so
the agent resolver called MiniMax with a Kimi model id. The provider returned
HTTP 400 `unknown model 'kimi-for-coding'`.

The backend delivered an `Error(agent_error)` frame to the Web endpoint, but
`TimelineView` did not render `useChatSessionWs().error`, leaving the user
with only their own prompt bubble and no visible failure.

## Decisions

- A prompt frame may carry `model_provider` alongside `model`.
- A prompt frame with `model` and no `model_provider` clears any stale
  provider pin before dispatching the turn.
- Session WebSocket backend error frames render in the Chat UI near the
  composer.

## Acceptance Criteria

```gherkin
Feature: Chat model pins and LLM errors remain visible and coherent

  Scenario: WS model pin clears stale provider when provider is absent
    Given a session already has model="kimi-for-coding"
      and model_provider="minimax"
    When the browser sends a prompt frame with a model but no model_provider
    Then the session WebSocket clears the stale provider pin before dispatching the turn

    Test: crates/channels/tests/web_session_smoke.rs
    Filter: session_ws_prompt_with_model_override_pins_turn_model

  Scenario: Chat renders backend error frames
    Given the persistent session WebSocket reports an error frame
    When TimelineView renders the chat composer
    Then the error message is visible near the composer

    Test: web/src/components/topology/__tests__/TimelineView.history.test.tsx
    Filter: session_ws_error_frame_is_rendered_near_composer
```

## Boundaries

Allowed:

- `crates/channels/src/web_session.rs`
- `crates/channels/tests/web_session_smoke.rs`
- `web/src/agent/session-ws-client.ts`
- `web/src/agent/__tests__/session-ws-client.test.ts`
- `web/src/components/topology/TimelineView.tsx`
- `web/src/components/topology/__tests__/TimelineView.history.test.tsx`

## Out of Scope

- Global provider settings redesign.
- Production DB edits.
- Remote backend restart.
- Replacing the vendored input component.
