#!/usr/bin/env bash
# spec-lifecycle-guard.sh — zero-match guard around `agent-spec lifecycle`.
#
# agent-spec (<= 0.3.0) reports a scenario as passed even when the cargo test
# filter matches ZERO tests ("running 0 tests" / "0 passed; N filtered out").
# That false-green converts "unverified" into "verified" — see issue #2165 and
# the verbatim record in PR #2038. Upstream report:
# https://github.com/ZhangHanDong/agent-spec/issues/4
#
# This wrapper runs the lifecycle with JSON output, then inspects the raw
# runner output captured in each scenario's test evidence. A scenario whose
# test runs executed zero tests (0 passed AND 0 failed across every
# `test result:` line) is a FAIL, regardless of agent-spec's own verdict.
#
# `just spec-lifecycle` routes through this script; `just spec-selftest`
# asserts it rejects specs/fixtures/zero-match.spec.md.
#
# Usage: scripts/spec-lifecycle-guard.sh <spec-file>

set -uo pipefail

if [ $# -ne 1 ]; then
    echo "usage: $0 <spec-file>" >&2
    exit 2
fi
SPEC="$1"

for tool in agent-spec jq; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "spec-lifecycle-guard: '$tool' not found — run ./init.sh for install hints" >&2
        exit 2
    fi
done

REPORT="$(mktemp)"
trap 'rm -f "$REPORT"' EXIT

agent-spec lifecycle "$SPEC" --code . --change-scope worktree --format json >"$REPORT"
LIFECYCLE_EXIT=$?

# Human-readable summary of the JSON report (text format omits runner output,
# which is exactly what the guard needs — so we run JSON once and render it).
jq -r '
  "=== Lifecycle Report (guarded) ===",
  "Spec: \(.verification.spec_name // "unknown")  stage: \(.stage)  passed: \(.passed)",
  (.verification.results[]? | "  [\(.verdict | ascii_upcase)] \(.scenario_name)")
' "$REPORT" || cat "$REPORT"

if [ "$LIFECYCLE_EXIT" -ne 0 ]; then
    echo "spec-lifecycle-guard: agent-spec lifecycle exited $LIFECYCLE_EXIT" >&2
    exit "$LIFECYCLE_EXIT"
fi

# Zero-match detection: for every scenario with test evidence, sum executed
# tests (passed + failed) across all `test result:` lines in the captured
# runner stdout. Zero executed tests means the selector resolved to nothing.
ZERO_MATCH=$(jq -r '
  [ .verification.results[]?
    | select([.evidence[]? | select(.type == "test_output")] | length > 0)
    | { scenario: .scenario_name,
        executed: ([ .evidence[]?
                     | select(.type == "test_output")
                     | .stdout // ""
                     | scan("(\\d+) passed; (\\d+) failed")
                     | map(tonumber) | add
                   ] | add // 0) }
    | select(.executed == 0)
    | .scenario
  ] | .[]
' "$REPORT")

if [ -n "$ZERO_MATCH" ]; then
    echo "" >&2
    echo "spec-lifecycle-guard: FAIL — Test selector(s) matched ZERO tests (0 passed; filtered out):" >&2
    while IFS= read -r scenario; do
        [ -n "$scenario" ] && echo "  - $scenario" >&2
    done <<< "$ZERO_MATCH"
    echo "Every lane-1 Test: selector must resolve to >=1 real test function — see specs/README.md." >&2
    exit 1
fi

echo "spec-lifecycle-guard: OK — every Test selector executed >=1 test"
exit 0
