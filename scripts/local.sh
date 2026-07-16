#!/bin/sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$repo_root"

if [ -f .env.local ]; then
  set -a
  . ./.env.local
  set +a
fi

compose_env_args=""
if [ -f .env.local ]; then
  compose_env_args="--env-file .env.local"
fi

running_self_host_apps="$(
  docker compose ps backend dashboard agent-gateway --status running --format '{{.Service}}' 2>/dev/null || true
)"
if [ -n "$running_self_host_apps" ]; then
  printf '%s\n' "Stopping self-host app containers and keeping PostgreSQL running for local mode."
  docker compose stop backend dashboard agent-gateway >/dev/null
fi

shared_database_url=""
if docker compose ps postgres --status running --format '{{.Service}}' 2>/dev/null | grep -qx postgres; then
  shared_database_url="$(
    docker compose exec -T postgres sh -eu -c \
      'cat /run/vifu/secrets/database_url' 2>/dev/null || true
  )"
fi

if [ -n "$shared_database_url" ]; then
  DATABASE_URL="$(printf '%s\n' "$shared_database_url" | sed 's/@postgres:5432/@127.0.0.1:5432/')"
  export DATABASE_URL
  printf '%s\n' "Using the running self-host PostgreSQL container for local mode."
else
  printf '%s\n' "Starting local PostgreSQL container."
  docker compose $compose_env_args \
    -f self-hosted/docker/docker-compose.yml \
    -f self-hosted/docker/docker-compose.local.yml \
    up -d --wait postgres
fi

export VIFU_DEPLOYMENT_MODE=local
export VIFU_SERVER_ADDR="${VIFU_SERVER_ADDR:-127.0.0.1:6790}"
export VIFU_SERVER_URL="${VIFU_SERVER_URL:-http://127.0.0.1:6790}"
export VIFU_API_BASE_URL="${VIFU_API_BASE_URL:-http://127.0.0.1:6790}"
export NEXT_PUBLIC_VIFU_API_BASE_URL="${NEXT_PUBLIC_VIFU_API_BASE_URL:-http://localhost:6790}"
export VIFU_AUTH_MODE="${VIFU_AUTH_MODE:-none}"
export VIFU_HOME="${VIFU_HOME:-$HOME/.vifu}"

pids=""
cleanup() {
  for pid in $pids; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
}
trap cleanup INT TERM EXIT

cargo run -p vifu-server &
pids="$pids $!"

bun run --cwd npm-packages/dashboard dev &
pids="$pids $!"

cargo run -p vifu &
pids="$pids $!"

printf '%s\n' "Local Vifu is starting."
printf '%s\n' "Dashboard: http://localhost:6791"
printf '%s\n' "Server:    http://127.0.0.1:6790"
if [ -f "$VIFU_HOME/providers.json" ]; then
  printf '%s\n' "Providers: $VIFU_HOME/providers.json"
else
  printf '%s\n' "Providers: not configured"
fi
printf '%s\n' "Press Ctrl-C to stop the local server, dashboard, and gateway."

wait
