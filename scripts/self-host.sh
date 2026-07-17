#!/bin/sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$repo_root"

compose_env_args=""
if [ -f .env ]; then
  compose_env_args="--env-file .env"
fi

exec docker compose $compose_env_args up -d
