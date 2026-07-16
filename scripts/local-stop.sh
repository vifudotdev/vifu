#!/bin/sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$repo_root"

compose_env_args=""
if [ -f .env.local ]; then
  compose_env_args="--env-file .env.local"
fi

docker compose $compose_env_args \
  -f self-hosted/docker/docker-compose.yml \
  -f self-hosted/docker/docker-compose.local.yml \
  down
