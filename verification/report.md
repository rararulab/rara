# Verification report — issue #2218 — PASS

- **base_sha:** `b593f58196587cee98fbd5bb025360be81af7fea`
- **head_sha:** `9958562da3819ca5c8db3134445337c10bac3921`
- **lane:** 2 (chore, `ui`)
- **score_authority:** verifier — implementer evidence not read (`self_check_only`)

## Scope check
`git diff --stat origin/main..HEAD` touches only `web/src/` app code (ThemeToggle.tsx; ui/{alert-dialog,button,command,dialog,select,sheet,tabs}.tsx; index.css; pages/{Chat,Docs}.tsx). No `web/src/vendor/craft-ui/**` modified. No `crates/**`. Working tree clean.

## (a) Quality gate from clean state — all green
- `bun run typecheck` (`tsc -b --noEmit`) → EXIT 0
- `bun run lint` (`eslint .`) → EXIT 0
- `bun run build` (`tsc -b && vite build`) → EXIT 0, `✓ built in 10.39s` (only the pre-existing chunk-size warning)

## (b) Issue Verify
`cd web && bun run build` → passed above.

## Core acceptance — the previously-DEAD plugin is now live
`index.css` adds `@plugin "tailwindcss-animate";`. The built bundle `dist/assets/index-C_e3-Owx.css` now emits what was absent at base: `@keyframes enter`, `@keyframes exit`, `.animate-in{animation-name:enter}`, `.animate-out[data-state=closed]{animation-name:exit}`.

## (c)/(d) Runtime measurement (real Chromium via Playwright, real built CSS + real running app)

NORMAL motion (fine pointer):

| Surface | measured | verdict |
|---|---|---|
| Button `transition-property` | `color, background-color, box-shadow, transform` (not `all`) | PASS |
| Button `:active` (CDP forced pseudo) | `scale: 0.97` | PASS |
| Select | `animation-name: enter`; `transform-origin` consumes `--radix-select-content-transform-origin` | PASS |
| Dialog open / closed | `enter` @ 0.2s / `exit` @ 0.15s (exit faster) | PASS |
| AlertDialog | same `duration-200`/`duration-150` split | PASS |
| Sheet open / closed | `enter` @ 0.25s `cubic-bezier(0.22,1,0.36,1)` (was 0.5s ease-in-out) / `exit` @ 0.15s | PASS |
| Command palette | `enter` @ 0s (instant) | PASS |
| Docs card hover (fine) | `translate: 0px -1.875px` (lift applies) | PASS |

Real running app (Vite :51733): first live `<button>` computed `transition-property = color, background-color, box-shadow, transform` @ 0.15s — the primitive change is live in the real app, not just the harness.

## Probes (e)

| Probe | observed | verdict |
|---|---|---|
| `prefers-reduced-motion: reduce` | Select/Dialog/Sheet all `animation-name: none`, `0s`; Button transition retained (transition ≠ animation) | PASS |
| coarse pointer / touch (`hasTouch,isMobile`) | `(hover:hover) and (pointer:fine)` = false; docs hover `translate: none` — no lift, no stuck hover | PASS |
| CJK dialog body (long 中文 in `max-w-lg`) | scrollWidth 478 == clientWidth 478, `withinViewport: true`, wraps; `animation-name: enter` | PASS |

Interruptibility (rapid open/close) is Radix Presence mount/unmount behavior, untouched by this PR; exit is now faster (0.15s), only shrinking any stuck-state window — no regression introduced.

## Transition matrix
- **fail_to_pass:** at base_sha the Radix Content enter/exit animations emitted no CSS (plugin unregistered → keyframes/utilities absent); at head_sha they emit and play at runtime (measured `animation-name: enter/exit`). Plus: Button `:active` scale 0.97 + explicit transition list; Select honors trigger origin; Dialog/AlertDialog exit < enter; Sheet ease-out sub-300ms; Command instant; reduced-motion suppresses entrances; coarse-pointer suppresses docs lift.
- **pass_to_fail: 0** — no regressions.

## Verdict
PASS — plugin registered and every targeted surface's motion / press / reduced-motion behavior confirmed at runtime by measured computed styles; gate green from clean state; craft-ui boundary respected; no regressions. No repair round needed.
