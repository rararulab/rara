#!/usr/bin/env bash
# Reset the rara data directory used by the live Playwright suite.
#
# Live specs (web/e2e/chat.spec.ts) drive the real backend over HTTP/WS.
# They each create + delete their own session via the API, so no fixture
# data is needed beyond an empty database with all migrations applied.
#
# This script just wipes the SQLite DB; the next `rara server` boot
# replays every embedded migration into a fresh file.
#
# Required env vars:
#   XDG_DATA_HOME — points at the isolated data root (CI sets this to a
#                   temp dir; locally set it before invoking).
#
# Usage:
#   XDG_DATA_HOME=/tmp/rara-e2e/data ./scripts/e2e-seed.sh

set -euo pipefail

if [[ -z "${XDG_DATA_HOME:-}" ]]; then
  echo "error: XDG_DATA_HOME must be set so the seed targets an isolated data dir" >&2
  exit 1
fi

DB_DIR="${XDG_DATA_HOME}/rara/db"
mkdir -p "$DB_DIR"
rm -f "$DB_DIR/rara.db" "$DB_DIR/rara.db-wal" "$DB_DIR/rara.db-shm"
echo "e2e-seed: cleared $DB_DIR — next rara start will re-run migrations"
