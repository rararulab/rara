# Development Workflow — Issue → Worktree → PR → Merge

**Every code change — no matter how small — MUST follow this workflow.** There are zero exceptions: single-line fixes, typo corrections, config tweaks, doc updates, and refactors all go through issue + worktree + PR. The main agent must NEVER directly edit source files on the `main` branch.

```
1. CREATE ISSUE    →  gh issue create + labels
2. CREATE WORKTREE →  git worktree add .worktrees/issue-{N}-{name} -b issue-{N}-{name}
3. WORK            →  All edits happen inside the worktree
4. VERIFY          →  cargo check + npm run build on worktree
5. PUSH & PR       →  git push -u origin + gh pr create
6. CLEANUP         →  git worktree remove + git branch -d (after PR merged)
```

## Step 1: Create Issue
```bash
gh issue create --title "feat(kernel): event queue sharding" \
  --label "created-by:claude" --label "enhancement" --label "core"
```

**Issue Labels** (all issues MUST have proper labels):
- `created-by:claude` — required for all agent-created issues
- **Type** (pick one): `bug`, `enhancement`, `refactor`, `chore`, `documentation`
- **Component** (pick one): `core`, `backend`, `ui`, `extension`, `ci`

## Step 2: Create Worktree
```bash
git worktree add .worktrees/issue-{N}-{short-name} -b issue-{N}-{short-name}
```

## Step 3: Work in Worktree
- **All code edits happen exclusively inside the worktree directory** — never in the main checkout
- The main agent may dispatch a subagent to the worktree, or work there directly
- Independent issues can be dispatched **in parallel** (each in its own worktree)
- All work should be committed before moving to the next step

## Step 4: Verify Builds
After subagent completes, verify in the worktree:
```bash
cargo check -p {crate-name}   # Rust backend
cd web && npm run build        # Frontend (if touched)
```

## Pre-commit Checks (prek)

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
- `RUSTDOCFLAGS="-D warnings" cargo +nightly doc --workspace --no-deps --document-private-items`

Triggers on: `.rs`, `.toml`, `Cargo.lock`, `rust-toolchain.toml` changes.

Run all checks manually:
```bash
prek run --all-files           # Run all hooks
just pre-commit                # Alternative: fmt + clippy + check + test
```

If pre-commit hook blocks a commit during development, fix issues before the final commit. Do NOT use `--no-verify` to skip hooks.

## Step 5: Push & Create PR
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

## Step 5.5: Wait for CI Green (MANDATORY)

After creating the PR, **you MUST verify that all CI checks pass before reporting completion to the user.**

```bash
gh pr checks {PR-number} --watch    # Wait for all checks to complete
```

- If any check fails, investigate and fix in the worktree, push again, and re-verify
- Do NOT report "PR created" or "task done" to the user while CI is still pending or failing
- Only after all checks are green may you inform the user that the PR is ready

## Step 6: Cleanup (after PR merged)
```bash
git worktree remove .worktrees/issue-{N}-{short-name}
git branch -d issue-{N}-{short-name}
```

## Parallel Execution

When user requests involve multiple independent changes, split into separate issues and dispatch subagents in parallel:
- Each subagent gets its own worktree, branch, and PR
- PRs are reviewed and merged independently on GitHub
