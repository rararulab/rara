# Stacked PRs for Large Features

When a feature is too large for a single PR (> ~400 lines of change, or spans multiple crates/layers), use stacked PRs to break it into reviewable increments.

**When to use**: Only for large features that benefit from incremental decomposition. Small/independent tasks continue using the standard workflow (`@docs/workflow.md`).

```
1. CREATE EPIC ISSUE  →  gh issue create for the overall feature
2. CREATE FEATURE BRANCH  →  git branch feat/{name} main
3. DECOMPOSE  →  Create sub-issues for each incremental step
4. STACK  →  Each sub-issue branches off the previous one (not main)
5. INCREMENTAL PRs  →  Each sub-PR targets the previous branch in the stack
6. FINAL PR  →  One summary PR from feat/{name} → main for boss review
```

## Step 1: Create Epic Issue
```bash
gh issue create --title "feat(scope): large feature description" \
  --label "created-by:claude" --label "enhancement" --label "core"
```
This is the tracking issue (e.g., #100). All sub-issues reference it.

## Step 2: Create Feature Base Branch
```bash
git checkout main && git pull
git branch feat/{name} main
git push -u origin feat/{name}
```

## Step 3: Decompose into Sub-Issues
Create one issue per incremental step, referencing the epic:
```bash
gh issue create --title "feat(scope): step 1 — add data model (#100)" \
  --body "Part of #100" \
  --label "created-by:claude" --label "enhancement" --label "core"
```

## Step 4: Stack Branches and Work
Each sub-issue gets a worktree branching off the previous step:
```bash
# Step 1: branches from feat/{name}
git worktree add .worktrees/issue-101-step1 -b issue-101-step1 feat/{name}

# Step 2: branches from step 1 (after step 1 is done)
git worktree add .worktrees/issue-102-step2 -b issue-102-step2 issue-101-step1
```

## Step 5: Incremental PRs
Each sub-PR targets the previous branch in the stack:
```bash
# Step 1 PR: targets feat/{name}
gh pr create --base feat/{name} --title "feat(scope): step 1 — add data model (#101)" \
  --body "Part of #100. Closes #101" --label "enhancement" --label "core"

# Step 2 PR: targets step 1 branch
gh pr create --base issue-101-step1 --title "feat(scope): step 2 — add service layer (#102)" \
  --body "Part of #100. Closes #102" --label "enhancement" --label "core"
```
- Merge sub-PRs in order (step 1 first, then step 2, etc.)
- After merging a sub-PR, update the next PR's base if needed: `gh pr edit {N} --base feat/{name}`

## Step 6: Final Summary PR
After all sub-PRs are merged into `feat/{name}`, create the summary PR:
```bash
gh pr create --base main --head feat/{name} \
  --title "feat(scope): large feature description (#100)" \
  --body "Closes #100\n\n## Summary\n- Step 1: ...\n- Step 2: ...\n- Step 3: ..." \
  --label "enhancement" --label "core"
```
This is the **only PR the reviewer needs to look at** — a single, complete view of the entire feature.
