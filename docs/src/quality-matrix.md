# Module Quality Matrix

A living dashboard that tracks the health of every crate in the Rara workspace. Inspired by engineering maturity models, this matrix helps identify gaps in documentation, testing, and maintainability across the codebase.

**Legend:** ✅ Present/Good | ⚠️ Partial | ❌ Missing/None

> **Last updated:** 2026-03-18

## Quality Table

| Crate | Layer | AGENT.md | Tests | Docs | LOC | Notes |
|-------|-------|----------|-------|------|----:|-------|
| `rara-kernel` | kernel | ✅ | ✅ | ⚠️ 102/381 (27%) | 28,904 | Largest crate; doc coverage needs improvement |
| `rara-app` | app | ❌ | ✅ | ⚠️ 22/86 (26%) | 10,642 | High-traffic crate missing AGENT.md |
| `rara-channels` | app | ❌ | ✅ | ⚠️ 29/62 (47%) | 8,231 | Decent doc coverage |
| `rara-skills` | app | ✅ | ✅ | ⚠️ 25/81 (31%) | 4,487 | — |
| `rara-cli` | cmd | ❌ | ✅ | ❌ 2/48 (4%) | 3,899 | Binary crate; minimal docs |
| `common-worker` | common | ❌ | ❌ | ⚠️ 29/48 (60%) | 3,664 | Good docs but no tests |
| `rara-backend-admin` | extensions | ❌ | ❌ | ❌ 3/48 (6%) | 3,284 | No tests, no docs, no AGENT.md |
| `rara-mcp` | integrations | ❌ | ❌ | ⚠️ 6/29 (21%) | 3,389 | No tests |
| `rara-symphony` | app | ❌ | ✅ | ⚠️ 5/26 (19%) | 2,760 | — |
| `rara-dock` | app | ✅ | ✅ | ⚠️ 10/49 (20%) | 2,466 | — |
| `rara-soul` | app | ❌ | ✅ | ⚠️ 16/37 (43%) | 1,223 | — |
| `rara-composio` | integrations | ❌ | ❌ | ⚠️ 2/11 (18%) | 1,138 | — |
| `rara-vault` | app | ✅ | ✅ | ❌ 1/12 (8%) | 1,138 | Has AGENT.md and tests; docs lacking |
| `common-telemetry` | common | ❌ | ❌ | ⚠️ 10/24 (42%) | 1,073 | Observability infra without tests |
| `rara-server` | server | ❌ | ❌ | ⚠️ 7/24 (29%) | 1,060 | — |
| `base` | common | ❌ | ❌ | ❌ 2/24 (8%) | 906 | Foundational crate with poor docs |
| `rara-agents` | app | ❌ | ✅ | ✅ 5/5 (100%) | 479 | Small and well-documented |
| `yunara-store` | common | ❌ | ❌ | ❌ 0/12 (0%) | 460 | Zero doc coverage |
| `rara-git` | extensions | ❌ | ❌ | ⚠️ 3/8 (38%) | 460 | — |
| `rara-codex-oauth` | integrations | ❌ | ❌ | ✅ 20/22 (91%) | 426 | Excellent docs but no tests |
| `rara-paths` | common | ❌ | ❌ | ✅ 22/22 (100%) | 382 | Fully documented; needs tests and AGENT.md |
| `common-runtime` | common | ❌ | ❌ | ❌ 2/18 (11%) | 348 | — |
| `rara-sessions` | app | ❌ | ❌ | ❌ 1/4 (25%) | 312 | — |
| `rara-error` | common | ❌ | ❌ | ❌ 0/6 (0%) | 187 | — |
| `rara-domain-shared` | domain | ❌ | ❌ | ❌ 0/11 (0%) | 186 | — |
| `rara-tool-macro` | common | ✅ | ❌ | ❌ 0/1 (0%) | 185 | Proc-macro crate |
| `crawl4ai` | common | ❌ | ❌ | ❌ 0/2 (0%) | 158 | — |
| `rara-keyring-store` | integrations | ❌ | ❌ | ❌ 0/5 (0%) | 107 | — |
| `rara-pg-credential-store` | integrations | ❌ | ❌ | ❌ 0/1 (0%) | 79 | — |
| `rara-model` | domain | ❌ | ❌ | — | 20 | Mostly migrations; minimal Rust code |

## Aggregate Statistics

| Metric | Count | Percentage |
|--------|------:|----------:|
| **Total crates** | 30 | — |
| **With AGENT.md** | 6 | 20% |
| **With tests** | 13 | 43% |
| **Doc coverage > 50%** | 4 | 13% |
| **Total Rust LOC** | ~82,714 | — |

### By Layer

| Layer | Crates | Avg Doc Coverage | With AGENT.md | With Tests |
|-------|-------:|-----------------:|--------------:|-----------:|
| common | 8 | 18% | 1 | 0 |
| domain | 2 | 0% | 0 | 0 |
| kernel | 1 | 27% | 1 | 1 |
| app | 8 | 33% | 3 | 7 |
| server | 1 | 29% | 0 | 0 |
| cmd | 1 | 4% | 0 | 1 |
| extensions | 2 | 17% | 0 | 0 |
| integrations | 5 | 26% | 0 | 0 |

## Priority Actions

1. **AGENT.md gap**: 24 of 30 crates lack an AGENT.md. High-priority targets: `rara-app` (10K LOC), `rara-channels` (8K LOC), `rara-server`, and `common-worker`.
2. **Test gap**: 17 crates have zero tests. Critical gaps: `rara-mcp` (3.4K LOC), `rara-backend-admin` (3.3K LOC), `common-worker` (3.7K LOC), and `rara-server` (1K LOC).
3. **Doc coverage**: Only 4 crates exceed 50% doc coverage. The kernel crate (29K LOC, 27%) is the highest-impact target for improvement.
4. **Common layer**: The foundational `common/` crates have the worst overall quality — no tests, sparse docs, and almost no AGENT.md files. Since every other layer depends on them, improving common/ has outsized impact.
