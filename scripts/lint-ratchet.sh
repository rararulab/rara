#!/usr/bin/env bash
# lint-ratchet.sh — freeze existing lint debt, fail on ANY increase (#2174).
#
# The ratcheted lints below have hundreds of pre-existing findings, so they
# stay `allow` in [workspace.lints] (prek / CI clippy runs `-D warnings` and
# would go red on all of them). This script force-warns exactly those lints,
# counts findings per (lint, crate) — so a fix in crate A cannot mask new
# debt in crate B — and compares against the committed baseline:
#
#   count > baseline  -> FAIL (new debt)
#   count < baseline  -> PASS, and prints how to tighten the baseline
#   count == baseline -> PASS
#
# Generated code is excluded: OUT_DIR spans surface as absolute paths in
# cargo's JSON diagnostics, in-repo spans as relative ones.
#
# Usage:
#   scripts/lint-ratchet.sh                    # check against the baseline
#   scripts/lint-ratchet.sh --update-baseline  # rewrite the baseline (commit it)
#
# The baseline is shared across platforms; cfg-gated code can make a count
# platform-dependent. If CI reports an INCREASE your platform cannot see,
# take the union-max of both platforms' counts into the baseline.
#
# Graduation: when a lint hits 0 across all crates, delete its entries from
# the baseline, flip it to "warn" in Cargo.toml, and remove it from
# RATCHETED_LINTS here.

set -euo pipefail

# Keep in sync with the `# RATCHET` comments in [workspace.lints] (Cargo.toml).
RATCHETED_LINTS=("dead_code" "unreachable_pub" "clippy::too_many_lines")

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
baseline_file="$repo_root/scripts/lint-ratchet-baseline.json"

force_warn_args=()
for lint in "${RATCHETED_LINTS[@]}"; do
    force_warn_args+=("--force-warn" "$lint")
done

lints_json="$(printf '%s\n' "${RATCHETED_LINTS[@]}" | jq -R . | jq -sc .)"

echo "lint-ratchet: running clippy (force-warn: ${RATCHETED_LINTS[*]}) ..."

# --all-targets WITHOUT --all-features: matches the issue #2174 measurement
# and rust.yml's clippy job, and keeps optional native toolchains (zig) out
# of the loop. Diagnostics are deduplicated by span because --all-targets
# compiles lib code once per target kind and replays the same warning.
current_counts="$(
    cargo clippy --quiet --workspace --all-targets --message-format=json \
        -- "${force_warn_args[@]}" \
    | jq -sc --arg root "$repo_root" --argjson lints "$lints_json" '
        [ .[]
          | select(.reason == "compiler-message")
          | select(.message.code != null)
          | select(.message.code.code as $c | $lints | index($c))
          | . as $msg
          | ($msg.message.spans[] | select(.is_primary))
          | select(.file_name | startswith("/") | not)
          | select(.file_name | startswith("target/") | not)
          | { lint: $msg.message.code.code,
              crate: ($msg.manifest_path | ltrimstr($root + "/") | rtrimstr("/Cargo.toml")),
              file: .file_name, line: .line_start, col: .column_start }
        ]
        | unique
        | group_by([.lint, .crate])
        | map({ key: (.[0].lint + "|" + .[0].crate), value: length })
        | from_entries
    '
)"

if [[ "${1:-}" == "--update-baseline" ]]; then
    jq -S . <<<"$current_counts" >"$baseline_file"
    echo "lint-ratchet: baseline written to $baseline_file — review and commit it."
    exit 0
fi

if [[ ! -f "$baseline_file" ]]; then
    echo "lint-ratchet: missing baseline $baseline_file" >&2
    echo "lint-ratchet: run 'scripts/lint-ratchet.sh --update-baseline' and commit it." >&2
    exit 1
fi

# Fail-closed guard: if the baseline has entries for a lint that produced
# zero diagnostics in this run, the far more likely cause is a renamed /
# mistyped lint (RATCHETED_LINTS out of sync with the baseline, or an
# upstream rename making --force-warn a no-op) than 500+ findings paid off
# in one hop. Without this check such a breakage reads as an all-decrease
# pass. If the debt genuinely hit 0, graduate the lint instead: flip it to
# "warn" in Cargo.toml, drop it from RATCHETED_LINTS and the baseline.
missing_lints="$(
    jq -n -r --argjson cur "$current_counts" --slurpfile base_arr "$baseline_file" '
        ($base_arr[0] | keys | map(split("|")[0]) | unique) as $base_lints
        | ($cur | keys | map(split("|")[0]) | unique) as $cur_lints
        | ($base_lints - $cur_lints)[]
    '
)"
if [[ -n "$missing_lints" ]]; then
    echo "lint-ratchet: FAIL — baseline lints produced no diagnostics at all:" >&2
    sed 's/^/  /' <<<"$missing_lints" >&2
    echo "Check RATCHETED_LINTS in scripts/lint-ratchet.sh against the baseline;" >&2
    echo "if a lint truly reached 0, graduate it (Cargo.toml \"warn\" + remove here)." >&2
    exit 1
fi

diff_lines="$(
    jq -n -r --argjson cur "$current_counts" --slurpfile base_arr "$baseline_file" '
        $base_arr[0] as $base
        | ([($cur | keys[]), ($base | keys[])] | unique) as $keys
        | $keys[]
        | { key: ., cur: ($cur[.] // 0), base: ($base[.] // 0) }
        | select(.cur != .base)
        | "\(if .cur > .base then "INCREASE" else "decrease" end)\t\(.key)\t\(.base) -> \(.cur)"
    '
)"

status=0
if grep -q '^INCREASE' <<<"$diff_lines"; then
    status=1
    echo ""
    echo "lint-ratchet: FAIL — lint debt increased vs scripts/lint-ratchet-baseline.json:"
    grep '^INCREASE' <<<"$diff_lines" | sed 's/^/  /'
    echo ""
    echo "Fix the new findings (do not bump the baseline to absorb new debt)."
    echo "Reproduce locally with:"
    echo "  cargo clippy --workspace --all-targets --quiet -- $(printf -- '--force-warn %s ' "${RATCHETED_LINTS[@]}")"
fi

if grep -q '^decrease' <<<"$diff_lines"; then
    echo ""
    echo "lint-ratchet: debt went DOWN — nice. Tighten the ratchet:"
    grep '^decrease' <<<"$diff_lines" | sed 's/^/  /'
    echo ""
    echo "  scripts/lint-ratchet.sh --update-baseline   # then commit the baseline"
fi

if [[ $status -eq 0 ]]; then
    echo "lint-ratchet: OK — no lint count increased vs the baseline."
fi
exit $status
