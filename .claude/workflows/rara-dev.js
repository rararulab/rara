// rara-dev — rara's deterministic development workflow as a dynamic Workflow script.
//
// Replaces the parent agent's hand-chained, prose-driven orchestration (the
// "Auto-chaining" section of docs/guides/workflow.md) with real control flow:
// a true review loop, real fan-out over independent issues, and explicit stop
// points at human gates.
//
// The agent CONTRACTS live in harness/roles/*.md (engine-neutral), with
// .claude/agents/*.md as thin dispatch wrappers — this script only
// orchestrates. Each agent() call dispatches the corresponding agentType, so
// there is no prompt duplication: spec-author / implementer-backend /
// implementer-frontend / implementer / verifier / reviewer are the single
// source of truth. Stage protocol: harness/stages.toml; normative design:
// docs/guides/pipeline.md.
//
// HARD BOUNDARY: a Workflow runs headless in the background and cannot pause to
// ask the user anything. rara's merge-to-main is gate (a) — a human gate — so
// this script runs all the way to "PR open + CI green" and STOPS. The parent
// agent reads the result and walks each PR through the merge confirmation.
//
// Invoke: Workflow({ name: 'rara-dev', args: { request: "<verbatim user request>" } })
//
// RESUME (background workflows do NOT survive a Claude Code restart — the run is
// a child of the CLI session and is killed with it): if a run dies mid-flight,
// relaunch with Workflow({ scriptPath, resumeFromRunId }). Completed agent() calls
// (e.g. spec-author) return cached results; the interrupted stage re-runs. The
// implement stage is idempotent (it reuses an existing worktree), so resume is safe.

export const meta = {
  name: 'rara-dev',
  description: "rara deterministic dev workflow: spec -> fan-out -> implement -> verify -> review-loop -> push -> PR -> CI green (stops before merge gate)",
  phases: [
    { title: 'Spec', detail: 'spec-author gates against goal.md, prior-art search, splits into independent issues' },
    { title: 'Implement', detail: 'one implementer per issue: worktree + code + quality gate + local commit' },
    { title: 'Verify', detail: 'fresh-context verifier (S3, harness/roles/verifier.md): clean-state gate + cold boot + hostile probes; FAIL -> one repair round -> escalate' },
    { title: 'Review', detail: 'reviewer <-> implementer loop until APPROVE (max 3 rounds), before push; fix commits invalidate the verify verdict and trigger a one-shot re-verify' },
    { title: 'Ship', detail: 'push + gh pr create (verification report path in PR body) + gh pr checks --watch (real CI gate); stops before merge' },
  ],
}

// ---- input ----------------------------------------------------------------

const REQUEST = (args && typeof args === 'object') ? args.request : args
if (!REQUEST || typeof REQUEST !== 'string') {
  throw new Error('rara-dev requires args.request — the verbatim user feature/bug request string.')
}
const MAX_REVIEW_ROUNDS = 3
// The verify stage's repair budget is exactly ONE round (issue #2164): verify
// FAIL -> one structured repair dispatch -> re-verify -> still FAIL -> stop and
// escalate to human. Distinct from the review loop's 3-round cap above.
const MAX_VERIFY_REPAIR_ROUNDS = 1
// 'watch' (default — real CI on GitHub-hosted runners, #2166): ship runs
//   `gh pr checks --watch` and requires the required checks (Rust Success /
//   Lint Success) to go green.
// 'signoff': EMERGENCY OVERRIDE ONLY (e.g. GitHub-hosted runner outage) —
//   ship pushes + opens the PR + runs `gh signoff` instead of watching CI.
//   Requires branch protection to have been temporarily flipped back to
//   ["signoff"] per the procedure in docs/guides/workflow.md. Explicit
//   opt-in via args.ci; never the default.
// 'skip': no GitHub-level gate at all — ship stops after PR creation;
//   the implement-stage local gate is the only verification.
const CI_MODE = (args && typeof args === 'object' && args.ci) ? args.ci : 'watch'
if (!['watch', 'signoff', 'skip'].includes(CI_MODE)) {
  throw new Error(`rara-dev: unknown ci mode ${JSON.stringify(CI_MODE)} — expected 'watch', 'signoff', or 'skip'.`)
}
if (CI_MODE === 'signoff') {
  log('⚠️  CI_MODE=signoff — EMERGENCY OVERRIDE. Real CI (gh pr checks --watch) is being bypassed; ' +
      'this is only valid during a GitHub-hosted runner outage with branch protection flipped to ' +
      '["signoff"] per docs/guides/workflow.md. If CI is healthy, drop the ci arg and use watch.')
}
if (CI_MODE === 'skip') {
  log('⚠️  CI_MODE=skip — NO GitHub-level gate at all (no CI watch, no signoff). The implement-stage ' +
      "local quality gate is the only verification; merging is entirely the user's call.")
}

// ---- schemas --------------------------------------------------------------

const PLAN_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['summary', 'issues'],
  properties: {
    summary: { type: 'string', description: 'What spec-author decided: lane split, scope, key decisions.' },
    issues: {
      type: 'array',
      description: 'INDEPENDENT issues, one-issue-one-PR. If the request is not independently splittable, return exactly one issue.',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['issueNumber', 'title', 'slug', 'lane', 'variant', 'specPath', 'allowedPaths'],
        properties: {
          issueNumber: { type: 'integer', description: 'The GitHub issue number actually filed.' },
          title: { type: 'string' },
          slug: { type: 'string', description: 'kebab-case short name for worktree/branch (issue-N-<slug>).' },
          lane: { type: 'string', enum: ['lane-1', 'lane-2'] },
          variant: { type: 'string', enum: ['backend', 'frontend', 'generic'], description: 'backend=crates/**, frontend=web/** or extension/**, generic=docs/repo-root/.claude.' },
          specPath: { type: ['string', 'null'], description: 'specs/issue-N-<slug>.spec.md for lane-1, null for lane-2.' },
          allowedPaths: { type: 'array', items: { type: 'string' }, description: 'Glob roots the implementer may touch.' },
        },
      },
    },
  },
}

const IMPL_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['committed', 'worktreePath', 'commits', 'outcome'],
  properties: {
    committed: { type: 'boolean', description: 'true only if the quality gate passed and a local commit exists.' },
    worktreePath: { type: 'string', description: 'Absolute or repo-relative .worktrees/issue-N-<slug> path.' },
    commits: { type: 'array', items: { type: 'string' }, description: 'Local commit SHAs (not pushed).' },
    outcome: { type: 'string', description: 'Concrete outcome verification — evidence the change works, not a restatement.' },
    blockers: { type: 'string', description: 'Why committed=false, if applicable. Empty otherwise.' },
  },
}

const VERIFY_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['verdict', 'reportPath', 'baseSha', 'headSha', 'summary'],
  properties: {
    verdict: { type: 'string', enum: ['PASS', 'FAIL'], description: 'PASS only if gate green from clean state, spec-lifecycle / Verify commands green, end-to-end drive observed, pass_to_fail == 0.' },
    reportPath: { type: 'string', description: 'Path to verification/report.md inside the worktree.' },
    baseSha: { type: 'string', description: 'merge-base with origin/main at verification time.' },
    headSha: { type: 'string', description: 'Worktree HEAD the verdict binds to. A new commit invalidates the verdict.' },
    summary: { type: 'string', description: 'One-paragraph verdict rationale. On FAIL: the failing commands / probe inputs, verbatim.' },
  },
}

const VERDICT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['approved', 'findings', 'notes'],
  properties: {
    approved: { type: 'boolean', description: 'true only if NO P0/P1 findings remain and (lane-1) the spec review passes.' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['severity', 'where', 'problem'],
        properties: {
          severity: { type: 'string', enum: ['P0', 'P1', 'P2'] },
          where: { type: 'string', description: 'file:line or area.' },
          problem: { type: 'string' },
        },
      },
    },
    notes: { type: 'string', description: 'Spec-review result (lane-1) + regression-decision check summary.' },
  },
}

const SHIP_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['pushed', 'prNumber', 'prUrl', 'ciGreen', 'ciSummary'],
  properties: {
    pushed: { type: 'boolean' },
    prNumber: { type: ['integer', 'null'] },
    prUrl: { type: ['string', 'null'] },
    ciGreen: { type: 'boolean', description: 'true only after gh pr checks --watch reports all required checks passing.' },
    ciSummary: { type: 'string', description: 'Which checks passed/failed.' },
  },
}

// ---- helpers --------------------------------------------------------------

const VARIANT_TYPE = { backend: 'implementer-backend', frontend: 'implementer-frontend', generic: 'implementer' }
const VARIANT_MD = { backend: 'implementer-backend', frontend: 'implementer-frontend', generic: 'implementer' }
// In 'signoff' mode (emergency override during a CI outage) the local gate
// REPLACES workspace CI, so backend runs the full `cargo nextest run
// --workspace` (what rust.yml gates) instead of just the touched crate —
// otherwise cross-crate regressions slip through. In the default 'watch'
// mode, remote CI covers --workspace, so the local gate stays narrow
// (`cargo test -p <crate>`). Driver-stack e2e (e2e.yml) and the Linux matrix
// remain CI-only — uncovered during an outage by design.
const BACKEND_TEST = CI_MODE === 'signoff'
  ? 'cargo nextest run --workspace (full workspace — replaces the down CI; if nextest is unavailable, cargo test --workspace)'
  : 'cargo test -p <crate>'
const VARIANT_GATE = {
  backend: `cargo check / cargo +nightly fmt --check / cargo clippy -D warnings / prek run --all-files / ${BACKEND_TEST}`,
  frontend: 'bun run build / TS typecheck / ESLint + the make-interfaces-feel-better self-review',
  generic: 'prek run --all-files',
}
const variantType = (issue) => VARIANT_TYPE[issue.variant] ?? 'implementer'

const branchOf = (i) => `issue-${i.issueNumber}-${i.slug}`
const worktreeOf = (i) => `.worktrees/${branchOf(i)}`

// Appended to every agent prompt. A workflow cannot advance until the agent
// emits its structured result, so make that requirement explicit — a heavy
// agentType (e.g. implementer) can otherwise end its turn with prose only and
// hang the agent() call forever.
const EMIT = `

IMPORTANT: finish by calling the StructuredOutput tool with the required schema. Do NOT end your turn with prose only — the workflow blocks until you emit the structured result. Emit it as your final action even if some steps were skipped (record what was skipped in the result).`

function implementPrompt(issue) {
  const wt = worktreeOf(issue)
  const lane1 = issue.lane === 'lane-1'
  return `You are rara's implementer for GitHub issue #${issue.issueNumber} ("${issue.title}").
Follow your full contract in .claude/agents/${VARIANT_MD[issue.variant]}.md.

This step is IDEMPOTENT — it may run on a RESUMED workflow, so first reconcile state:
0. \`git worktree list | grep ${branchOf(issue)}\`.
   - If the worktree exists AND \`git -C ${wt} log origin/main..HEAD\` already shows a commit that satisfies this issue: re-run the quality gate to confirm it still passes, then return the existing worktree path + commit SHAs. Do NOT redo the work.
   - If it does not exist: \`git worktree add ${wt} -b ${branchOf(issue)}\` from the main checkout.
   Do ALL edits inside ${wt} — never on main.

Otherwise implement:
1. Read \`gh issue view ${issue.issueNumber}\`${lane1 ? ` and the spec \`${issue.specPath}\`` : ''}.
2. Implement the SMALLEST change that satisfies the ${lane1 ? 'spec' : 'issue'}, touching ONLY these allowed paths: ${issue.allowedPaths.join(', ')}.
3. Run the ${issue.variant} quality gate: ${VARIANT_GATE[issue.variant]}.${lane1 ? `\n4. Run \`just spec-lifecycle ${issue.specPath}\` — every BDD scenario must report pass (no skip, no uncertain).` : ''}
${lane1 ? '5' : '4'}. Commit LOCALLY in the worktree: Conventional Commit subject + "Closes #${issue.issueNumber}" in the body. Do NOT push.

Set committed=true ONLY if the gate passed and a local commit exists. Put concrete outcome verification in \`outcome\` (evidence, not a restatement of the task).` + EMIT
}

// FRESH CONTEXT by construction: this prompt carries the worktree path, the
// issue number, and the lane — and deliberately NOT impl.outcome or any other
// implementer evidence. The verifier's value is exactly that it never saw the
// implementation story (score authority: only it may emit `verified`).
function verifyPrompt(issue, impl, attempt) {
  const lane1 = issue.lane === 'lane-1'
  return `You are rara's verifier (S3${attempt > 1 ? `, re-verification after a repair round` : ''}) for GitHub issue #${issue.issueNumber}.
Follow your full contract in .claude/agents/verifier.md (-> harness/roles/verifier.md). You are a FRESH context: you have not seen the implementation and must not ask for it — derive what to verify from the issue${lane1 ? ' and the spec' : ''} alone.

Inputs: worktree \`${impl.worktreePath}\`, issue #${issue.issueNumber}, lane ${issue.lane}${lane1 ? `, spec \`${issue.specPath}\`` : ` (read the issue's Verify: commands from \`gh issue view ${issue.issueNumber}\`)`}.

Do, per the contract:
(a) re-run the full quality gate from clean state in the worktree; record base_sha and head_sha;
(b) ${lane1 ? `run \`just spec-lifecycle ${issue.specPath}\` — every scenario must pass` : `run the issue's Verify: commands verbatim`};
(c) if the change has a runtime surface, cold-boot the candidate build (\`just portless-run\`, temp data dir — NEVER a running instance) and drive the changed feature end-to-end;
(d) run 2-3 hostile probes (CJK input, empty/boundary values, concurrency);
(e) write \`verification/report.md\` in the worktree (base_sha, head_sha, score_authority: verifier, commands with raw outputs, transition matrix, verdict).

Return verdict PASS or FAIL with the report path. FAIL summaries must contain the failing commands / probe inputs verbatim — they become the repair contract.` + EMIT
}

function repairPrompt(issue, impl, verify) {
  return `You are rara's implementer addressing an S3 verification FAIL for issue #${issue.issueNumber} in worktree \`${impl.worktreePath}\`.
Follow .claude/agents/${VARIANT_MD[issue.variant]}.md. This is the ONLY repair round — a second FAIL escalates to a human.

The independent verifier's failing findings (report: ${verify.reportPath}):
${verify.summary}

Do:
1. Read the full report at ${verify.reportPath} in the worktree.
2. Fix the root cause with NEW commits (no amend). Any failing probe input MUST land as a regression test, not just be patched.
3. Re-run the ${issue.variant} quality gate${issue.lane === 'lane-1' ? ` and \`just spec-lifecycle ${issue.specPath}\`` : ''}.

Do NOT claim "verified" — your evidence is self_check_only; the verifier re-verifies from scratch. Return the updated result (committed=true if the gate passes again, with the new commit SHAs appended).` + EMIT
}

function reviewPrompt(issue, impl, round, verify) {
  const lane1 = issue.lane === 'lane-1'
  return `You are rara's reviewer (round ${round}/${MAX_REVIEW_ROUNDS}) for issue #${issue.issueNumber}, BEFORE push.
Follow your full contract in .claude/agents/reviewer.md. The implementer worked in worktree \`${impl.worktreePath}\` (commits: ${impl.commits.join(', ') || 'see git log'}).
The S3 verification report (verdict PASS, score_authority: verifier) is at \`${verify?.reportPath ?? '<worktree>/verification/report.md'}\` — a required review input (harness/stages.toml S4); read it alongside the diff.

Do:
1. Read \`git -C ${impl.worktreePath} diff origin/main..HEAD\`.
${lane1 ? `2. Run \`agent-spec lint\` + \`agent-spec lifecycle\` on ${issue.specPath}; do the CRITICAL spec review (aligns with goal.md? scenarios non-vacuous and actually falsify the Intent? Boundaries narrow?).\n` : ''}3. Run the generalized cross-file regression-decision check: \`git log --since=30.days\` on every file the diff touches; flag any re-introduction of what a recent PR removed.
4. Run the /code-review-expert checks. Inspect the implementer's outcome evidence — does it verify the outcome or only a side effect?

Return the verdict. approved=true ONLY if there are no P0/P1 findings${lane1 ? ' and the spec review passes' : ''}. List every finding with severity.` + EMIT
}

function fixPrompt(issue, impl, verdict) {
  return `You are rara's implementer addressing review findings for issue #${issue.issueNumber} in worktree \`${impl.worktreePath}\`.
Follow .claude/agents/${VARIANT_MD[issue.variant]}.md. Fix every P0/P1 finding with NEW commits (no amend), then re-run the ${issue.variant} quality gate${issue.lane === 'lane-1' ? ` and \`just spec-lifecycle ${issue.specPath}\`` : ''}.

Findings to address:
${verdict.findings.map(f => `- [${f.severity}] ${f.where}: ${f.problem}`).join('\n')}
${verdict.notes ? `\nReviewer notes: ${verdict.notes}` : ''}

Return the updated result (committed=true if the gate passes again, with the new commit SHAs appended).` + EMIT
}

function shipPrompt(issue, impl, verify) {
  const wt = impl.worktreePath
  const step3 =
    CI_MODE === 'watch'
      ? `\`gh pr checks <PR-number> --watch\`. If a check fails, diagnose the root cause, fix in ${wt}, push again (cap genuine-flake reruns at 1). Do NOT mark tests ignored to go green.\n\nSTOP after CI is green. Set ciGreen=true only when all required checks pass.`
      : CI_MODE === 'signoff'
        ? `EMERGENCY OVERRIDE (CI outage): branch protection has been temporarily flipped so main's only required check is \`signoff\` (see docs/guides/workflow.md). Do NOT run \`gh pr checks --watch\` (it would hang). The local quality gate already passed in the implement stage, so run \`gh signoff\` to sign off the pushed commit — this satisfies the required check and makes the PR mergeable. signoff binds to the commit: if you pushed again after the last gate run, re-run the gate then \`gh signoff\` again.\n\nSTOP after signoff succeeds. Set ciGreen=true (signoff is the green signal) and put "CI outage override; signed off after local gate" in ciSummary.`
        : `GitHub CI is UNAVAILABLE and signoff is not required — do NOT run \`gh pr checks --watch\`. Stop after the PR is created. Set ciGreen=false and put "no GitHub gate; local quality gate passed in implement stage" in ciSummary.`
  return `You are rara's implementer shipping issue #${issue.issueNumber} from worktree \`${wt}\`. The verifier PASSED (S3) and the reviewer APPROVED — push is unlocked.

Do:
1. \`git -C ${wt} push -u origin ${branchOf(issue)}\`.
2. \`gh pr create --base main --title "<conventional subject> (#${issue.issueNumber})" --body "<per .github/pull_request_template.md, include Closes #${issue.issueNumber}>" --label <type> --label <component>\`.
   The PR body MUST include a "Verification" section with the S3 report path and verdict:
   \`Verification: PASS — ${verify?.reportPath ?? '<worktree>/verification/report.md'} (base ${verify?.baseSha ?? '<base_sha>'}, head ${verify?.headSha ?? '<head_sha>'}; score_authority: verifier)\`.
3. ${step3}

Do NOT merge — merge-to-main is human gate (a). Return prNumber, prUrl, ciGreen, and ciSummary.` + EMIT
}

// Shared S3 runner: one verification plus at most MAX_VERIFY_REPAIR_ROUNDS
// repair rounds. Used by the verify stage AND by the post-review re-verify —
// a verify PASS binds to verify.headSha, so any commit landed after it (e.g.
// review-round fixes) invalidates the verdict (pipeline.md "Parallel-run
// rules": a new commit moves head_sha; a stale PASS must never ride into S5).
async function runVerify(issue, impl, labelPrefix) {
  let verify = await agent(
    verifyPrompt(issue, impl, 1),
    { agentType: 'verifier', label: `${labelPrefix}:a1`, phase: 'Verify', schema: VERIFY_SCHEMA },
  )
  for (let repair = 1; verify.verdict !== 'PASS' && repair <= MAX_VERIFY_REPAIR_ROUNDS; repair++) {
    const fixed = await agent(
      repairPrompt(issue, impl, verify),
      { agentType: variantType(issue), label: `${labelPrefix}:repair${repair}`, phase: 'Verify', schema: IMPL_SCHEMA },
    )
    if (fixed && fixed.committed) impl = fixed
    verify = await agent(
      verifyPrompt(issue, impl, repair + 1),
      { agentType: 'verifier', label: `${labelPrefix}:a${repair + 1}`, phase: 'Verify', schema: VERIFY_SCHEMA },
    )
  }
  if (verify.verdict !== 'PASS') {
    return { impl, verify, verified: false, escalate: true, reason: `verify FAIL after ${MAX_VERIFY_REPAIR_ROUNDS} repair round(s) — human decision needed: ${verify.summary}` }
  }
  return { impl, verify, verified: true }
}

// ---- orchestration --------------------------------------------------------

phase('Spec')
const plan = await agent(
  `You are rara's spec-author. The user's verbatim request:\n\n"""\n${REQUEST}\n"""\n\n` +
  `Follow your full contract in .claude/agents/spec-author.md:\n` +
  `1. Read goal.md and gate the request against it.\n` +
  `2. Run the MANDATORY prior-art search (gh issue list, gh pr list, git log --grep, rg) — do not skip.\n` +
  `3. Write a private reproducer (the concrete bug that appears if we do nothing). If none can be written, the request is too vague — say so in summary and return zero issues.\n` +
  `4. Pick the lane per the single test (can a Test: selector bind to a real test that fails-before/passes-after?).\n` +
  `5. Split into INDEPENDENT issues (one-issue-one-PR, NO stacked work). If it is not independently splittable, return exactly ONE issue.\n` +
  `6. File each GitHub issue with labels agent:claude + type + component. For lane-1, write specs/issue-N-<slug>.spec.md and reference it in the issue body.\n\n` +
  `Return the structured plan with the issue numbers you actually filed.` + EMIT,
  { agentType: 'spec-author', phase: 'Spec', schema: PLAN_SCHEMA }
)

if (!plan.issues || plan.issues.length === 0) {
  return { aborted: true, reason: 'spec-author filed no issues (request too vague or rejected by goal.md gate).', summary: plan.summary }
}
log(`spec-author filed ${plan.issues.length} issue(s): ${plan.issues.map(i => `#${i.issueNumber}(${i.lane}/${i.variant})`).join(', ')}`)

// Fan out: every independent issue flows through implement -> verify -> review-loop
// -> ship on its own. pipeline() means issue A can be in review while issue B
// still implements — no barrier wasted.
const results = await pipeline(
  plan.issues,

  // stage 1 — implement in an isolated worktree, local commit only
  (issue) => agent(
    implementPrompt(issue),
    { agentType: variantType(issue), label: `impl:#${issue.issueNumber}`, phase: 'Implement', schema: IMPL_SCHEMA },
  ).then((impl) => ({ issue, impl })),

  // stage 2 — independent verification (S3): fresh-context verifier with score
  // authority. FAIL -> ONE structured repair round -> re-verify -> still FAIL ->
  // stop this issue and escalate to human (no second repair round).
  async (prev) => {
    const { issue, impl } = prev
    if (!impl || !impl.committed) {
      return { issue, impl, verified: false, reason: `implement failed: ${impl?.blockers || 'no commit'}` }
    }
    const v = await runVerify(issue, impl, `verify:#${issue.issueNumber}`)
    return { issue, ...v }
  },

  // stage 3 — reviewer <-> implementer loop until APPROVE (real loop, capped).
  // Review-round fix commits move HEAD past verify.headSha, so an APPROVE after
  // fixes triggers ONE re-verify (same one-repair budget) before ship — a stale
  // PASS must never ride into S5.
  async (prev) => {
    const { issue } = prev
    let { impl, verify } = prev
    if (!prev.verified) {
      return { issue, impl, verify, approved: false, escalate: prev.escalate ?? false, reason: prev.reason }
    }
    let approved = false
    let rounds = 0
    let lastVerdict = null
    let changedSinceVerify = false
    for (let round = 1; round <= MAX_REVIEW_ROUNDS; round++) {
      const verdict = await agent(
        reviewPrompt(issue, impl, round, verify),
        { agentType: 'reviewer', label: `review:#${issue.issueNumber}:r${round}`, phase: 'Review', schema: VERDICT_SCHEMA },
      )
      if (verdict.approved) {
        approved = true
        rounds = round
        break
      }
      lastVerdict = verdict
      if (round === MAX_REVIEW_ROUNDS) break
      const fixed = await agent(
        fixPrompt(issue, impl, verdict),
        { agentType: variantType(issue), label: `fix:#${issue.issueNumber}:r${round}`, phase: 'Review', schema: IMPL_SCHEMA },
      )
      if (fixed && fixed.committed) {
        impl = fixed
        changedSinceVerify = true
      }
    }
    if (!approved) {
      return { issue, impl, verify, approved: false, reason: `not APPROVED after ${MAX_REVIEW_ROUNDS} rounds`, verdict: lastVerdict }
    }
    if (changedSinceVerify) {
      const rv = await runVerify(issue, impl, `reverify:#${issue.issueNumber}`)
      impl = rv.impl
      verify = rv.verify
      if (!rv.verified) {
        return { issue, impl, verify, approved: false, escalate: true, reason: `post-review ${rv.reason}` }
      }
      // A re-verify repair round may itself land commits the reviewer has not
      // seen; those are verifier-mandated regression-test fixes, accepted
      // without another review round to keep the loop bounded. The PR diff
      // shows them to the human at the merge gate.
    }
    return { issue, impl, verify, approved: true, rounds }
  },

  // stage 4 — push + PR (verification report path in body) + CI watch;
  // STOP before merge (gate a)
  async (prev) => {
    const { issue, impl, verify, approved, escalate, reason } = prev
    if (!approved) return { issue: issue.issueNumber, shipped: false, ciGreen: false, escalated: escalate ?? false, reason }
    const ship = await agent(
      shipPrompt(issue, impl, verify),
      { agentType: variantType(issue), label: `ship:#${issue.issueNumber}`, phase: 'Ship', schema: SHIP_SCHEMA },
    )
    return { issue: issue.issueNumber, slug: issue.slug, shipped: ship?.pushed ?? false, verifyReport: verify?.reportPath ?? null, ...ship }
  },
)

const clean = results.filter(Boolean)
// 'skip': "ready" means PR open + pushed + local gate passed. 'watch' and
// 'signoff' both require ciGreen — in signoff mode `gh signoff` sets it true.
const isReady = (r) => r.prNumber && (CI_MODE === 'skip' ? r.shipped : r.ciGreen)
const readyToMerge = clean.filter(isReady)
const blocked = clean.filter((r) => !isReady(r))

return {
  summary: plan.summary,
  total: plan.issues.length,
  ci_mode: CI_MODE,
  ready_to_merge: readyToMerge.map((r) => ({ issue: r.issue, pr: r.prNumber, url: r.prUrl, ci: r.ciSummary, verify_report: r.verifyReport ?? null })),
  blocked: blocked.map((r) => ({ issue: r.issue, pr: r.prNumber ?? null, escalated: r.escalated ?? false, reason: r.reason ?? r.ciSummary ?? 'CI not green' })),
  gate: CI_MODE === 'signoff'
    ? 'STOPPED before merge. EMERGENCY OVERRIDE mode: each ready PR has been `gh signoff`-ed (local quality gate from the implement stage is the green signal), satisfying the temporarily-flipped required check. Restore the real required checks per docs/guides/workflow.md once the outage ends. merge-to-main is human gate (a): the parent confirms each PR with the user, then `gh pr merge --squash --delete-branch`, then cleans up the worktree.'
    : CI_MODE === 'skip'
      ? 'STOPPED after PR creation. GitHub CI is UNAVAILABLE and signoff not required — the only verification is the LOCAL quality gate (prek/cargo/bun) from the implement stage. merge-to-main is human gate (a); merging without any GitHub gate is entirely the user\'s call.'
      : 'STOPPED before merge. merge-to-main is human gate (a): the parent confirms each PR with the user, then `gh pr merge --squash --delete-branch`, then cleans up the worktree.',
}
