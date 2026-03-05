#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/docker-compose.agent.yml"

# By default this also clears compose volumes and local images.
exec docker compose -f "$COMPOSE_FILE" down --remove-orphans --volumes --rmi local
