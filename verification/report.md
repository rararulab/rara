# Verification report — issue #2224 — VERDICT: PASS

chore(site): honor prefers-reduced-motion + motion polish on landing page

- **base_sha:** `682da7b47da16e1fb86b8fdde2e1c674f92c7b07`
- **head_sha:** `b1aaead8c522a83a5f2a9f05f5c2b44469bd130b`
- **lane:** 2 (chore)
- **score_authority:** verifier
- **implementer_evidence:** self_check_only (not read)

## Scope check
`git diff --stat origin/main..HEAD`: 3 files, +95/−129 — `site/src/pages/index.astro` (+103/−8), deleted `site/src/components/Features.astro` (−75) + `Typewriter.astro` (−46).
- Touches ONLY `site/` — no `web/**`, no `crates/**`. PASS.
- `site/package-lock.json` unchanged; no `site/bun.lock` committed. PASS.
- `grep -rn "Features\|Typewriter" site/src` → NO REFS. PASS.

## (a) Quality gate from clean state
- `cd site && bun install` → no changes
- `cd site && bun run build` → astro build: 1 page built, "Complete!" (exit 0)
- `prek run --all-files` → all hooks Passed
Only build warning is an upstream `@astrojs/internal-helpers` node_modules notice — unrelated.

## (b) Lane-2 Verify command
`cd site && bun run build` → SUCCESS (orphan deletions did not break the static build).

## (c)/(d) Cold-boot + real-browser drive
Cold-booted `bun run preview` (http://localhost:4321/rara). Playwright headless Chromium, `emulateMedia({ reducedMotion })`, viewport 1280×800. "Animating" = differing `canvas.toDataURL()` SHA-256 frame hashes; "static" = identical hashes.

| Surface | Mode | Metric | Observed | Expected | Result |
|---|---|---|---|---|---|
| Hero `c0` | normal | distinct/5 (mouse-move) | 5 | >1 | PASS |
| Kernel `c1` | normal | distinct/5 | 5 | >1 | PASS |
| `.scroll-hint` | normal | `animation-name` | `bob` | `bob` | PASS |
| Hero `c0` | reduce | distinct/6 | 1 | 1 | PASS |
| Kernel `c1` | reduce | distinct/5 | 1 | 1 | PASS |
| Kernel `c1` | reduce | non-BG ink px | 347 | >0 | PASS |
| Kernel `c1` | reduce | screenshot | dashed kernel ring + 6 labeled agents at rest on evenly-spaced orbits — deliberate arrangement, not frozen mid-explosion | deliberate + legible | PASS |
| `.scroll-hint` | reduce | `animation-name` | `none` | `none` | PASS |
| All 4 panels text | reduce | screenshot | fully legible | legible | PASS |

### Probes
1. **Live toggle → reduce, NO reload:** `c1` distinct 4→1 (identical to fresh-reduced static frame); hero also went static. PASS.
2. **Live toggle back → no-preference, NO reload:** `c1` distinct=5, motion resumed (bi-directional). PASS.
3. **Orphan deletion + narrow viewport under reduce:** build succeeds, no references remain, copy legible at 420×720. PASS.

## Transition matrix
- **fail_to_pass:** at base_sha the page ignored `prefers-reduced-motion` entirely; at head_sha the hero physics/lens, kernel orbit, scroll-slide, and `bob` are all suppressed under reduce and content stays legible.
- **pass_to_fail:** 0. Normal-motion behavior fully preserved.

## Verdict
PASS — build + prek green from clean state; scope confined to `site/`, no stray lockfiles, no orphan imports; real-browser cold-boot proves normal motion animates, reduced motion is static + legible across all four panels, and the live `matchMedia` `change` listener stops and resumes motion bi-directionally without reload. No repair round needed.
