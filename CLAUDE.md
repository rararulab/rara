# CLAUDE.md — Rara Development Guide

## Communication
- 用中文与用户交流

## Project Identity

Rara is a self-evolving, developer-first personal proactive agent built in Rust. It uses a kernel-inspired architecture with heartbeat-driven proactive behavior, 3-layer memory, and a skills system.

## Development Workflow

### Issue → Worktree → PR → Merge (MANDATORY)

**Every code change — no matter how small — MUST follow this workflow.** There are zero exceptions: single-line fixes, typo corrections, config tweaks, doc updates, and refactors all go through issue + worktree + PR. The main agent must NEVER directly edit source files on the `main` branch.

```
1. CREATE ISSUE    →  gh issue create + labels
2. CREATE WORKTREE →  git worktree add .worktrees/issue-{N}-{name} -b issue-{N}-{name}
3. WORK            →  All edits happen inside the worktree
4. VERIFY          →  cargo check + npm run build on worktree
5. PUSH & PR       →  git push -u origin + gh pr create
6. CLEANUP         →  git worktree remove + git branch -d (after PR merged)
```

#### Step 1: Create Issue
```bash
gh issue create --title "feat(kernel): event queue sharding" \
  --label "created-by:claude" --label "enhancement" --label "core"
```

**Issue Labels** (all issues MUST have proper labels):
- `created-by:claude` — required for all agent-created issues
- **Type** (pick one): `bug`, `enhancement`, `refactor`, `chore`, `documentation`
- **Component** (pick one): `core`, `backend`, `ui`, `extension`, `ci`

#### Step 2: Create Worktree
```bash
git worktree add .worktrees/issue-{N}-{short-name} -b issue-{N}-{short-name}
```

#### Step 3: Work in Worktree
- **All code edits happen exclusively inside the worktree directory** — never in the main checkout
- The main agent may dispatch a subagent to the worktree, or work there directly
- Independent issues can be dispatched **in parallel** (each in its own worktree)
- All work should be committed before moving to the next step

#### Step 4: Verify Builds
After subagent completes, verify in the worktree:
```bash
cargo check -p {crate-name}   # Rust backend
cd web && npm run build        # Frontend (if touched)
```

#### Pre-commit Checks (prek)

The project uses [prek](https://github.com/j178/prek) for pre-commit hooks. The **final commit** in any PR must pass all checks — intermediate commits during development don't need to pass.

Setup (required once after clone):
```bash
brew install prek              # Install prek
prek install                   # Install git hooks into .git/hooks
```

Hooks configured in `.pre-commit-config.yaml`:
- `cargo check --all --all-targets`
- `cargo +nightly fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings`

Triggers on: `.rs`, `.toml`, `Cargo.lock`, `rust-toolchain.toml` changes.

Run all checks manually:
```bash
prek run --all-files           # Run all hooks
just pre-commit                # Alternative: fmt + clippy + check + test
```

If pre-commit hook blocks a commit during development, fix issues before the final commit. Do NOT use `--no-verify` to skip hooks.

#### Step 5: Push & Create PR
```bash
git push -u origin issue-{N}-{short-name}
gh pr create --title "fix(scope): description" --body "Closes #{N}" \
  --label "bug" --label "core"
```
- Commit message must include `Closes #N` so the issue is auto-closed when PR merges
- Never merge locally — all merges happen through GitHub PR
- **PR Labels** (all PRs MUST have proper labels):
  - **Type** (pick one): `bug`, `enhancement`, `refactor`, `chore`, `documentation`
  - **Component** (pick one): `core`, `backend`, `ui`, `extension`, `ci`
  - Note: a `labeler.yml` workflow auto-labels PRs by file path, but agents must still add type + component labels explicitly via `--label` flags

#### Step 5.5: Wait for CI Green (MANDATORY)

After creating the PR, **you MUST verify that all CI checks pass before reporting completion to the user.**

```bash
gh pr checks {PR-number} --watch    # Wait for all checks to complete
```

- If any check fails, investigate and fix in the worktree, push again, and re-verify
- Do NOT report "PR created" or "task done" to the user while CI is still pending or failing
- Only after all checks are green may you inform the user that the PR is ready

#### Step 6: Cleanup (after PR merged)
```bash
git worktree remove .worktrees/issue-{N}-{short-name}
git branch -d issue-{N}-{short-name}
```

### Parallel Execution

When user requests involve multiple independent changes, split into separate issues and dispatch subagents in parallel:
- Each subagent gets its own worktree, branch, and PR
- PRs are reviewed and merged independently on GitHub

### Database Migrations

- **Location**: `crates/rara-model/migrations/`（集中式）
- **永远不要修改已应用的迁移** — SQLx 通过 checksum 追踪，任何改动都会破坏启动
- Schema 变更必须创建**新迁移**，即使是修复上一个迁移的错误
- 使用 `just migrate-add <scope>_<description>` 创建迁移（如 `chat_add_pinned`）
- 本地数据库损坏时用 `just migrate-reset` 重建
- **不要在 Rust 代码中硬编码数据库默认值** — 所有配置通过 YAML 配置文件注入（`~/.config/job/config.yaml`）

### Commit Style
- Conventional commits: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`
- Scope matches crate or area: `feat(kernel):`, `fix(web):`, `refactor(memory):`
- Include `(#N)` issue reference in commit message
- Include `Closes #N` in commit body

## What NOT To Do

- Do NOT put repository impls or routes in `yunara-store` — business logic stays in its own crates
- Do NOT use manual `impl Display` + `impl Error` — use `snafu`
- Do NOT use mock repositories in tests — use `testcontainers`
- Do NOT work directly on `main` — ALL changes (code, docs, config) require a worktree + PR, no exceptions
- Do NOT merge locally on `main` — all merges go through GitHub PRs; never `git merge` or `git commit` on main
- Do NOT edit files in the main checkout for 'quick fixes' — even one-line changes must go through the full issue → worktree → PR flow
- Do NOT create issues without `created-by:claude` label
- Do NOT create PRs or issues without type + component labels — every PR and issue must have a type label (`bug`, `enhancement`, `refactor`, `chore`, `documentation`) and a component label (`core`, `backend`, `ui`, `extension`, `ci`)
- Do NOT leave stale worktrees — clean up after PR is merged
- Do NOT modify already-applied migration files — create a new migration instead
- Do NOT hardcode database URLs or config defaults in Rust code — use the YAML config file
- Do NOT use noop/hollow trait implementations to糊弄编译器 — trait method 有真正实现时不允许默认空体（silently return `Ok(())` / `Ok(None)` / `vec![]`）；可选 UX hook（`typing_indicator`, lifecycle hooks）是唯一例外
- Do NOT 构造空壳身份对象 — `Principal` 必须通过 `SecuritySubsystem::resolve_principal()` 或 `Principal::from_user()` 从数据库获得完整的 role + permissions，不允许用 placeholder 值存入 Session
- Do NOT 在 agent system prompt 中添加"先说计划再行动"类规则 — "先发 plan 再执行" 会导致 LLM 在简单交互（hello）中也产生冗余/重复的叙述文本。正确原则是 "act first, report after"（参见 #201）
- Do NOT 用过于宽泛的条件触发 memory search — "proactively search memory" 会让每次交互都触发搜索+无意义叙述。触发条件必须明确限定（如"用户明确问到过去的事"）
- Do NOT 修改 agent system prompt 后不测试 — 至少用 "hello"、"你好" 等简单输入验证不会产生异常/重复输出
- Do NOT 在 CI 未全绿的情况下向用户汇报"PR 已完成" — 必须用 `gh pr checks --watch` 等到所有 check 通过，失败则修复后重新推送并再次验证
