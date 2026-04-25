# web/src/hooks — Agent Guidelines

## Purpose

Custom React hooks shared across rara web pages.

## Architecture

Real files in this directory (one line each):

- `loading-hints.ts` — poetic placeholder strings + `randomLoadingHint()`, mirrors the Telegram channel's loading copy so both feel cohesive.
- `use-dock-store.ts` — in-memory dock state machine (sessions, blocks, mutations, history) for the agent dock UI.
- `use-live-card-height.ts` — `ResizeObserver`-driven publisher of the in-progress agent live-card height to a CSS custom property on `<main>`. Currently unused after pi-web-ui's retirement; kept around as a building block in case the new chat ever reintroduces a floating in-progress overlay.
- `use-local-storage.ts` — typed `useState`-style wrapper over `window.localStorage` with JSON serialization.
- `use-server-status.ts` — context hook exposing `{ isOnline, isChecking }` for the global server-status banner.
- `use-session-delete.ts` — wires the pure `decidePostDeleteAction` helper into the chat page's switch/create-new side effects so the dispatch is unit-testable.
- `use-session-timeline.ts` — react-query backed timeline state for a chat session: turns → timeline items, live-state tracking, loading hints.
- `use-theme.ts` — `useSyncExternalStore`-based theme hook (`light` / `dark` / `system`) with `localStorage` persistence and `prefers-color-scheme` subscription.

## Critical Invariants

- Hooks that observe DOM elements which mount conditionally (e.g. behind a guard like `!isInitializing`) MUST take the element as a parameter (via `useState` + callback ref in the parent), not a `useRef`. `useRef` mutations don't re-run effects, so a ref-based observer attaches only on the lucky render where the element exists. `useLiveCardHeight` is the canonical example.
- Any hook that writes a CSS custom property MUST clear it on unmount, so removing the consumer doesn't leak the value into the next session.

## What NOT To Do

- Do NOT use `useRef<HTMLElement>` for elements that are conditionally mounted — the observer will silently miss the late mount. Use `useState` + a callback ref so the effect re-runs when the element appears.
- Do NOT call `setState` from inside `ResizeObserver` / `MutationObserver` callbacks without batching (`requestAnimationFrame` or React 18 auto-batching). Synchronous re-renders inside the observer cause infinite loops.

## Dependencies

Nothing exotic — `react`, native DOM observer APIs (`ResizeObserver`, `MutationObserver`, `matchMedia`), `@tanstack/react-query` for server-state hooks.
