#!/usr/bin/env bash
# Validate commit message follows Conventional Commits format.
# https://www.conventionalcommits.org/
#
# Called by prek as a commit-msg hook. $1 is the commit message file.

set -euo pipefail

COMMIT_MSG_FILE="$1"
COMMIT_MSG=$(head -1 "$COMMIT_MSG_FILE")

# Allow merge commits and revert commits
if echo "$COMMIT_MSG" | grep -qE '^(Merge |Revert )'; then
    exit 0
fi

# Conventional Commits pattern:
#   type(scope): description
#   type: description
#   type(scope)!: description  (breaking change)
#
# Allowed types: feat, fix, refactor, docs, test, chore, ci, perf, style, build, revert
PATTERN='^(feat|fix|refactor|docs|test|chore|ci|perf|style|build|revert)(\([a-z0-9_-]+\))?!?: .+'

if ! echo "$COMMIT_MSG" | grep -qE "$PATTERN"; then
    echo "❌ Commit message does not follow Conventional Commits format."
    echo ""
    echo "  Expected: <type>(<scope>): <description>"
    echo "  Got:      $COMMIT_MSG"
    echo ""
    echo "  Allowed types: feat, fix, refactor, docs, test, chore, ci, perf, style, build, revert"
    echo "  Examples:"
    echo "    feat(kernel): add event queue sharding"
    echo "    fix(web): resolve hydration mismatch"
    echo "    docs: update CLAUDE.md workflow section"
    echo ""
    exit 1
fi
