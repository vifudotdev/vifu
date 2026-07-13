#!/bin/sh
set -eu

compose_file="self-hosted/docker/docker-compose.yml"
state_dir="$(mktemp -d "${TMPDIR:-/tmp}/vifu-e2e.XXXXXX")"
managed_stack=0
compose_project=""
browser_dashboard_port=""

free_port() {
  node -e 'const net=require("node:net");const server=net.createServer();server.listen(0,"127.0.0.1",()=>{console.log(server.address().port);server.close();});'
}

if [ -n "${VIFU_E2E_ENV_FILE:-}" ]; then
  env_file="$VIFU_E2E_ENV_FILE"
else
  env_file="$state_dir/.env"
  sh scripts/init-self-hosted.sh "$env_file" >/dev/null
  server_port="$(free_port)"
  dashboard_port="$(free_port)"
  browser_dashboard_port="$dashboard_port"
  postgres_port="$(free_port)"
  {
    printf '%s\n' "VIFU_SERVER_PORT=$server_port"
    printf '%s\n' "VIFU_DASHBOARD_PORT=$dashboard_port"
    printf '%s\n' "POSTGRES_PORT=$postgres_port"
  } >> "$env_file"
  compose_project="vifu-e2e-$$"
  managed_stack=1
fi

if [ ! -f "$env_file" ]; then
  printf '%s\n' "VIFU_E2E_ENV_FILE does not exist: $env_file" >&2
  exit 1
fi
set -a
. "$env_file"
set +a
if [ -z "$browser_dashboard_port" ]; then
  browser_dashboard_port="${VIFU_DASHBOARD_PORT:-6791}"
fi
if [ -z "${VIFU_ADMIN_KEY:-}" ]; then
  printf '%s\n' "VIFU_ADMIN_KEY is missing. Run sh scripts/init-self-hosted.sh first." >&2
  exit 1
fi
if [ -n "${VIFU_E2E_OPENCLAW_PORT:-}" ]; then
  openclaw_port="$VIFU_E2E_OPENCLAW_PORT"
else
  openclaw_port="$(free_port)"
fi
agent_gateway_log="$state_dir/agent-gateway.log"
mock_log="$state_dir/openclaw.log"
state_path="$state_dir/state.json"
mock_pid=""
agent_gateway_pid=""
use_existing_openclaw="${VIFU_E2E_USE_EXISTING_OPENCLAW:-0}"

compose() {
  if [ -n "$compose_project" ]; then
    docker compose -p "$compose_project" --env-file "$env_file" -f "$compose_file" "$@"
  else
    docker compose --env-file "$env_file" -f "$compose_file" "$@"
  fi
}

cleanup_processes() {
  if [ -n "$agent_gateway_pid" ]; then kill "$agent_gateway_pid" 2>/dev/null || true; fi
  if [ -n "$mock_pid" ]; then kill "$mock_pid" 2>/dev/null || true; fi
}

on_failure() {
  status=$?
  if [ "$status" -ne 0 ]; then
    printf '%s\n' "--- Agent Gateway log ---"
    tail -n 100 "$agent_gateway_log" 2>/dev/null || true
    printf '%s\n' "--- OpenClaw mock log ---"
    tail -n 100 "$mock_log" 2>/dev/null || true
    compose logs --no-color --tail=100 backend dashboard postgres || true
  fi
  cleanup_processes
  if [ "$managed_stack" = "1" ]; then compose down --volumes --remove-orphans >/dev/null 2>&1 || true; fi
  rm -rf -- "$state_dir"
  exit "$status"
}
trap on_failure EXIT INT TERM

if [ "$managed_stack" = "1" ]; then
  compose up -d --build --wait
  export VIFU_E2E_API_URL="http://127.0.0.1:$VIFU_SERVER_PORT"
  export VIFU_E2E_DASHBOARD_URL="http://127.0.0.1:$VIFU_DASHBOARD_PORT"
fi

if [ "$use_existing_openclaw" != "1" ]; then
  OPENCLAW_MOCK_PORT="$openclaw_port" node scripts/mock-openclaw.mjs >"$mock_log" 2>&1 &
  mock_pid=$!
fi

attempt=0
until curl --fail --silent "http://127.0.0.1:$openclaw_port/health" >/dev/null; do
  if [ -n "$mock_pid" ] && ! kill -0 "$mock_pid" 2>/dev/null; then
    cat "$mock_log" >&2
    exit 1
  fi
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 30 ]; then exit 1; fi
  sleep 1
done

VIFU_HOME="$state_dir/vifu-home" \
VIFU_AGENT_GATEWAY_TOKEN="${VIFU_E2E_AGENT_GATEWAY_TOKEN:-$VIFU_AGENT_GATEWAY_TOKEN}" \
cargo run -p vifu -- \
  --openclaw-url "http://127.0.0.1:$openclaw_port" \
  --server-url "${VIFU_E2E_API_URL:-http://127.0.0.1:6790}" \
  >"$agent_gateway_log" 2>&1 &
agent_gateway_pid=$!

attempt=0
until curl --fail --silent \
  -H "Authorization: Bearer ${VIFU_E2E_ADMIN_KEY:-$VIFU_ADMIN_KEY}" \
  "${VIFU_E2E_API_URL:-http://127.0.0.1:6790}/v1/agent-gateways" \
  | grep -q '"status":"connected"'; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 60 ]; then exit 1; fi
  sleep 1
done

if [ "$use_existing_openclaw" = "1" ]; then
  VIFU_E2E_STATE_PATH="$state_path" node scripts/test-self-hosted-e2e.mjs setup
else
  VIFU_E2E_OPENCLAW_MOCK_URL="http://127.0.0.1:$openclaw_port" \
  VIFU_E2E_STATE_PATH="$state_path" \
  node scripts/test-self-hosted-e2e.mjs setup
fi
compose restart postgres backend dashboard

attempt=0
until curl --fail --silent "${VIFU_E2E_API_URL:-http://127.0.0.1:6790}/v1/status" >/dev/null; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 60 ]; then exit 1; fi
  sleep 1
done

attempt=0
until curl --fail --silent "${VIFU_E2E_DASHBOARD_URL:-http://127.0.0.1:6791}/dashboard" >/dev/null; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 60 ]; then exit 1; fi
  sleep 1
done

attempt=0
until curl --fail --silent \
  -H "Authorization: Bearer ${VIFU_E2E_ADMIN_KEY:-$VIFU_ADMIN_KEY}" \
  "${VIFU_E2E_API_URL:-http://127.0.0.1:6790}/v1/agent-gateways" \
  | grep -q '"status":"connected"'; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 60 ]; then exit 1; fi
  sleep 1
done

VIFU_E2E_STATE_PATH="$state_path" node scripts/test-self-hosted-e2e.mjs verify
VIFU_SELF_HOSTED_E2E_DASHBOARD_URL="http://0.0.0.0:$browser_dashboard_port" \
VIFU_SELF_HOSTED_E2E_AUTH_EMAIL="${VIFU_E2E_AUTH_EMAIL:-admin@self-hosted.example}" \
VIFU_SELF_HOSTED_E2E_AUTH_PASSWORD="${VIFU_E2E_AUTH_PASSWORD:-correct horse battery staple}" \
npx playwright test --config playwright.self-hosted.config.ts
VIFU_E2E_STATE_PATH="$state_path" node scripts/test-self-hosted-e2e.mjs cleanup

cleanup_processes
agent_gateway_pid=""
mock_pid=""
if [ "$managed_stack" = "1" ]; then compose down --volumes --remove-orphans >/dev/null; fi
rm -rf -- "$state_dir"
trap - EXIT INT TERM
printf '%s\n' "Self-hosted Agent Gateway, persistence, and concurrency E2E passed."
