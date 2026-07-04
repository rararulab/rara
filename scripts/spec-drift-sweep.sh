#!/usr/bin/env bash
# spec-drift-sweep.sh — check that every `Test:` selector in
# specs/issue-*.spec.md still resolves to >=1 test function in the current
# workspace (spec drift baseline, issue #2165).
#
# Skips specs/fixtures/** by construction (the glob below only matches real
# task specs; fixtures intentionally violate resolution — that is what they
# test). Selectors with `Package: web` are reported as SKIP: there is no
# vitest adapter yet (issue #2015).
#
# Exit code: 0 when every checked selector resolves, 1 otherwise.

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

DRIFTED=0
CHECKED=0
SKIPPED=0

for spec in specs/issue-*.spec.md; do
    [ -e "$spec" ] || continue
    # Package/Filter pairs appear in this order under each Test: block.
    while IFS=$'\t' read -r pkg filter; do
        [ -n "$pkg" ] && [ -n "$filter" ] || continue
        if [ "$pkg" = "web" ]; then
            echo "SKIP   $spec :: $pkg :: $filter (no vitest adapter — issue #2015)"
            SKIPPED=$((SKIPPED + 1))
            continue
        fi
        CHECKED=$((CHECKED + 1))
        if out=$(cargo test -p "$pkg" "$filter" -- --list 2>&1); then
            matches=$(printf '%s\n' "$out" | grep -c ': test$')
            if [ "${matches:-0}" -gt 0 ]; then
                echo "OK     $spec :: $pkg :: $filter (${matches} match(es))"
            else
                echo "DRIFT  $spec :: $pkg :: $filter (resolves to zero tests)"
                DRIFTED=$((DRIFTED + 1))
            fi
        else
            echo "DRIFT  $spec :: $pkg :: $filter (cargo test --list failed: unknown package or build error)"
            DRIFTED=$((DRIFTED + 1))
        fi
    done < <(awk '
        /^[[:space:]]*Package:[[:space:]]*/ { pkg = $2 }
        /^[[:space:]]*Filter:[[:space:]]*/  { print pkg "\t" $2 }
    ' "$spec")
done

echo
echo "spec-drift-sweep: ${CHECKED} checked, ${DRIFTED} drifted, ${SKIPPED} skipped (web)"
[ "$DRIFTED" -eq 0 ]
