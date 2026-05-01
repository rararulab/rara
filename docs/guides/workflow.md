# Development Workflow — Spec / Issue → Worktree → Local Commit → Review → Push → PR → Merge

**Every code change — no matter how small — MUST follow this workflow.**
Single-line fixes, typo corrections, config tweaks, doc updates, and refactors
all go through the workflow below. The main agent must NEVER directly edit
source files on the `main` branch.

There are now two **lanes**, and one major change to the old flow:
**review happens BEFORE push, gating it.** The implementer commits locally,
the reviewer reads the worktree diff, and only on APPROVE does the code
leave your machine.

```
Lane 1 (spec-driven — feature, bugfix, anything with testable behavior):
  0. SPEC AUTHOR    →  spec-author writes specs/issue-N-<slug>.spec.md
                       + opens GitHub issue referencing it
  1. WORKTREE       →  parent creates .worktrees/issue-N-<slug>
                       and dispatches implementer
  2. IMPLEMENT      →  implementer reads spec; codes; runs prek + lifecycle;
                       commits LOCALLY (does not push)
  3. REVIEW         →  reviewer reads worktree diff + spec; verdict
                       (loop until APPROVE)
  4. PUSH + PR      →  implementer pushes; gh pr create; gh pr checks --watch
  5. MERGE          →  gh pr merge --squash --delete-branch (when CI green)
  6. CLEANUP        →  git worktree remove + git branch -D

Lane 2 (lightweight chore — structural, cleanup, CI, rename, config):
  0. SPEC AUTHOR    →  spec-author writes the GitHub issue body directly
                       (Intent + prior art + decisions + boundaries; no
                       BDD scenarios; no specs/*.spec.md file)
  1-6. same as lane 1 minus the spec file and minus `agent-spec lifecycle`
```

## Picking the lane

`spec-author` makes this call. The single test:

> Can I write at least one `Test:` selector that binds to a real test
> function — one that fails before the change and passes after?

- Yes → **lane 1**.
- No → **lane 2**.

If unsure, lane 2 (overhead-on-the-side-of-less). Lane 1's value is the
BDD binding to a real test; without that binding, lane 1 produces ceremony.

See `specs/README.md` for the full lane decision criteria.

## Step 0: spec-author

`spec-author` is invoked **before any issue exists**. The parent agent
hands the user's request (verbatim) to spec-author. Spec-author:

1. Reads `goal.md` to gate the request.
2. Runs the mandatory prior-art search (`gh issue list`, `gh pr list`,
   `git log --grep`, `rg`). This is the wall PR #1941 walked through
   unchallenged — do not skip.
3. For vague requests, asks 1–3 multi-choice clarifying questions.
4. Writes a private reproducer ("if we don't do this, this concrete bug
   appears: 1. … 2. … 3. observed bad outcome"). If no reproducer can be
   written, the request is too vague — escalate, do not proceed.
5. Picks the lane.
6. Drafts: lane 1 → `specs/issue-TBD-<slug>.spec.md`; lane 2 → issue body.
7. Files the GitHub issue with `agent:claude` + type + component labels.
   For lane 1, renames the spec from `issue-TBD-` to `issue-N-` once the
   issue number is assigned, and references the spec path in the issue body.

See `.claude/agents/spec-author.md` for the full contract.

## Auto-chaining

Once the user has acknowledged the proposed plan, the parent agent chains
through the workflow steps mechanically: spec-author → worktree + implementer
→ reviewer → push → PR → merge. The rule is structured as a **whitelist**:
the only times the agent stops to re-ask are the gates enumerated below.
Anything not on the list runs without re-asking — including, explicitly,
the cases that have historically tripped the agent into sycophantic
re-confirmation.

### Confirmation gates (exhaustive)

The parent agent stops and asks the user **only** in these cases:

- **(a) Merging to `main`.** The final gate before code lands. Always ask,
  even when CI is green and review is APPROVE'd.
- **(b) Destructive git operations.** `git reset --hard`, force-push,
  `branch -D` on shared branches, and any other operation that rewrites
  or discards committed history.
- **(c) Remote backend restart.** `pkill` or any other action that kills
  / restarts the rara backend on the shared remote (`raratekiAir`),
  because other people may be using the instance.

This list is closed. Adding a new gate is a separate user decision — do
not infer one from a single failure mode.

### Default-continue (no re-ask)

Everything else runs without a confirmation round-trip. The cases below
are named explicitly because they have actually tripped the agent into
re-asking; they are the rule, not exceptions:

- **Status queries mid-flow** — "进度?" / "where are we?" / "现在到哪一
  步了?". Answer the question; do not restate the plan and end with
  "要继续吗?".
- **Step transitions inside an already-approved plan** — spec-author →
  worktree + implementer → reviewer → push → PR. After spec-author
  returns an issue number, the parent dispatches the implementer
  **directly** — do not ask "要不要派 implementer 把它做掉？" / "should
  I dispatch implementer?". The plan was already approved; re-asking is
  sycophancy, not safety.
- **Re-dispatching a stalled subagent** — if a subagent stops mid-task,
  the parent re-dispatches with the carried-over context. No fresh
  approval needed.
- **Routine worktree / git tool calls inside an approved change** —
  `git add`, `git commit`, `git rebase origin/main` inside the worktree,
  `gh pr create`, `gh pr checks --watch`, `gh pr merge` (subject to
  gate (a)).
- **PR label adjustments** — adding / removing type / component labels
  on a PR the agent owns.

## Step 1: Worktree

```bash
git worktree add .worktrees/issue-{N}-{short-name} -b issue-{N}-{short-name}
```

The parent agent creates the worktree and then dispatches the right
**implementer variant** (see Step 2). The main agent never edits in-place
on `main` and never edits inside the main checkout — every edit is in a
worktree.

## Step 2: Implement (lane 1 and 2)

The implementer subagent comes in two stack-specific variants plus a
generic fallback. The parent picks based on the issue's allowed paths:

- **`implementer-backend`** — when the issue's `Boundaries.Allowed`
  (lane 1) or the file paths cited in the issue body (lane 2) are
  rooted in `crates/**`. Brings the Rust quality gate (`cargo check` /
  `cargo +nightly fmt` / clippy / `prek run --all-files` / `cargo test
  -p <crate>`), the snafu / bon / async-trait style anchors, the diesel
  migration discipline, and the #1907 config-schema guardrail. PR
  component label is one of `core` / `backend`.
- **`implementer-frontend`** — when the allowed paths are rooted in
  `web/**` or `extension/**`. Brings the bun-based gate (`bun run build`
  + ESLint), the `make-interfaces-feel-better` self-review, and the
  before/after screenshot evidence bar. Explicitly does NOT run cargo
  for FE-only diffs. PR component label is one of `ui` / `extension`.
- **`implementer`** (generic base) — fallback for issues that fit
  neither lane (pure docs, repo-root config, harness files like
  `.claude/**`). Runs only `prek run --all-files`.

For **mixed-stack issues** (touching both `crates/**` and `web/**`):
prefer to split at spec-author time into one BE issue + one FE issue.
If genuinely unsplittable (e.g. a new API endpoint plus its UI consumer
that must land atomically), the parent dispatches BE first then FE
serially against the **same** worktree, branch, and PR — each variant
runs only its own gate against its own part of the diff.

Whichever variant is dispatched, it:

1. Reads `gh issue view <N>`. For lane 1, also reads
   `specs/issue-N-<slug>.spec.md`.
2. Translates the request into a one-sentence outcome to verify, sends it
   back to the parent, and waits for ACK before coding. (This catches
   misalignment for the cost of a round-trip.)
3. Reads the actual code it will touch.
4. Implements the smallest change that satisfies the spec / issue.
5. Runs the **stack-specific quality gate** (see the variant's contract).
6. **Lane 1 only**: runs `just spec-lifecycle specs/issue-N-<slug>.spec.md`.
   Every BDD scenario must pass — no `skip`, no `uncertain`.
7. Commits locally. Conventional Commits subject + `Closes #N` in body.
8. **Does NOT push.** Reports back to the parent with the worktree path,
   commit SHAs, outcome verification (concrete evidence), and any
   decisions surfaced.

If the diff touches `crates/{app,kernel,channels,acp,sandbox}/src/`, the
backend variant adds or extends a Rust e2e test in the corresponding
`tests/` directory following `docs/guides/e2e-style.md` (lane 1 = no LLM,
lane 2 = scripted LLM via `ScriptedLlmDriver`, lane 3 = real LLM in
`e2e.yml`). If PR-time e2e coverage is infeasible, state in the PR body
which lane applies and why.

See `.claude/agents/implementer.md` for the shared base contract,
`.claude/agents/implementer-backend.md` for the Rust gate, and
`.claude/agents/implementer-frontend.md` for the FE gate.

### Pre-commit checks (prek)

The project uses [prek](https://github.com/j178/prek). Setup once:

```bash
brew install prek
prek install
```

Hooks (`.pre-commit-config.yaml`):

- `cargo check --all --all-targets`
- `cargo +nightly fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings`
- `RUSTDOCFLAGS="-D warnings" cargo +nightly doc --workspace --no-deps --document-private-items`

Manual run:

```bash
prek run --all-files
just pre-commit
```

The **final** commit must pass all checks. Intermediate commits during
development don't need to pass. Do NOT use `--no-verify` to skip hooks.

## Step 3: Review (BEFORE push — this is the new bit)

The parent dispatches the `reviewer` subagent against the worktree (not
the PR — the PR does not exist yet). The reviewer:

1. Reads `git -C <worktree> diff origin/main..HEAD`.
2. For lane 1: runs `agent-spec lint` + `agent-spec lifecycle` against the
   spec; runs the **critical spec review** (does the spec align with
   `goal.md`? are scenarios non-vacuous? do they actually falsify the
   Intent? are Boundaries narrow?).
3. Runs the **generalized cross-file regression-decision check** —
   `git log --since=30.days` on every file the diff touches, looking
   for prior commits that removed / restructured the same area. This
   is the generalized form of the #1907 lesson; it catches PR #1941's
   pattern (re-introducing what a recent PR explicitly removed).
4. Runs the standard `/code-review-expert` skill checks.
5. Inspects the implementer's outcome verification — is the evidence
   concrete? Does it verify the outcome, or only a side-effect?

Verdict:

- **REQUEST_CHANGES (P0/P1)**: implementer fixes in worktree (new commits,
  no amend), re-runs verification, hands back. Loop until APPROVE.
- **REQUEST_CHANGES on the spec itself (lane 1)**: escalate to spec-author
  via parent. Implementer does NOT silently fix the spec.
- **APPROVE**: implementer proceeds to step 4.

See `.claude/agents/reviewer.md` for the full contract.

## Step 4: Push + Open PR + Watch CI

Only after reviewer APPROVE:

```bash
git -C <worktree> push -u origin issue-{N}-{short-name}

gh pr create --base main \
  --title "<type>(<scope>): <description> (#N)" \
  --body "..." \
  --label "<type>" --label "<component>"

gh pr checks {PR-number} --watch
```

PR body uses `.github/pull_request_template.md`. Labels:

- **Type** (pick one): `bug`, `enhancement`, `refactor`, `chore`, `documentation`
- **Component** (pick one): `core`, `backend`, `ui`, `extension`, `ci`

Note: `labeler.yml` auto-labels by file path, but the implementer must
still add type + component labels explicitly via `--label`.

Commit message must include `Closes #N` so the issue auto-closes on merge.

If a CI check fails: read the failure log, diagnose root cause, fix in
the worktree, push again. Do not mark tests `#[ignore]` to make CI green.
For genuine flakes (same test failed recently on `main`):
`gh run rerun <id> --failed`. Cap reruns at 1.

**Why review-before-push:** CI catches platform issues (Linux ARC runner
behavior vs your local macOS) and integration regressions. Review catches
design issues, regression-decision reversals, and scope creep. They don't
catch the same things, but pushing only after review APPROVE means
PR-level CI runs on already-reviewed code — no force-pushes after review,
no PRs lingering with "needs another round of review" comments. The
trade-off: any platform-only failure is caught after push, which is fine
because it's typically a one-line fix.

## Step 5: Merge

Green CI + already-APPROVE'd review = merge.

```bash
gh pr merge {N} --squash --delete-branch
```

Use `--squash` so the merged commit on `main` matches the Conventional
Commit subject. `--delete-branch` removes the remote branch; the local
branch and worktree are removed in step 6.

The parent has standing approval; do not re-ask.

## Step 6: Cleanup

```bash
git worktree remove .worktrees/issue-{N}-{short-name}
git branch -D issue-{N}-{short-name}    # -D because the branch is gone on origin
```

## Parallel execution

When user requests involve multiple independent changes, split into
separate issues at step 0 and dispatch implementer subagents in parallel:

- Each subagent gets its own worktree, branch, and PR.
- PRs are reviewed and merged independently on GitHub.
- The reviewer runs per-PR; reviewers do not share context across parallel
  PRs.
