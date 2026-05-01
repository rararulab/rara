# web/src/hooks — Agent Guidelines

## Purpose

Custom React hooks shared across rara web pages.

## Architecture

Real files in this directory (one line each):

- `loading-hints.ts` — poetic placeholder strings + `randomLoadingHint()`, mirrors the Telegram channel's loading copy so both feel cohesive.
- `use-local-storage.ts` — typed `useState`-style wrapper over `window.localStorage` with JSON serialization.
- `use-server-status.ts` — context hook exposing `{ isOnline, isChecking }` for the global server-status banner.
- `use-session-timeline.ts` — react-query backed timeline state for a kernel session: turns → timeline items, live-state tracking, loading hints.
- `use-theme.ts` — `useSyncExternalStore`-based theme hook (`light` / `dark` / `system`) with `localStorage` persistence and `prefers-color-scheme` subscription.
- `use-topology-subscription.ts` — WebSocket subscription to the cross-session topology event stream backing the `/chat` page (old `/topology` route redirects here).

## Critical Invariants

- Hooks that observe DOM elements which mount conditionally (e.g. behind a guard like `!isInitializing`) MUST take the element as a parameter (via `useState` + callback ref in the parent), not a `useRef`. `useRef` mutations don't re-run effects, so a ref-based observer attaches only on the lucky render where the element exists.
- Any hook that writes a CSS custom property MUST clear it on unmount, so removing the consumer doesn't leak the value into the next session.

## What NOT To Do

- Do NOT use `useRef<HTMLElement>` for elements that are conditionally mounted — the observer will silently miss the late mount. Use `useState` + a callback ref so the effect re-runs when the element appears.
- Do NOT call `setState` from inside `ResizeObserver` / `MutationObserver` callbacks without batching (`requestAnimationFrame` or React 18 auto-batching). Synchronous re-renders inside the observer cause infinite loops.

## Dependencies

Nothing exotic — `react`, native DOM observer APIs (`ResizeObserver`, `MutationObserver`, `matchMedia`), `@tanstack/react-query` for server-state hooks.
