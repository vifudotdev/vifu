#!/bin/sh
set -eu

compose_file="self-hosted/docker/docker-compose.yml"
env_file="${VIFU_E2E_ENV_FILE:-.env}"
if [ -f "$env_file" ]; then
  set -a
  . "$env_file"
  set +a
fi
if [ -z "${VIFU_ADMIN_KEY:-}" ]; then
  printf '%s\n' "VIFU_ADMIN_KEY is missing. Run sh scripts/init-self-hosted.sh first." >&2
  exit 1
fi
if [ -n "${VIFU_E2E_OPENCLAW_PORT:-}" ]; then
  openclaw_port="$VIFU_E2E_OPENCLAW_PORT"
else
  openclaw_port="$(node -e 'const net=require("node:net");const server=net.createServer();server.listen(0,"127.0.0.1",()=>{console.log(server.address().port);server.close();});')"
fi
state_dir="$(mktemp -d "${TMPDIR:-/tmp}/vifu-e2e.XXXXXX")"
connector_log="$state_dir/connector.log"
mock_log="$state_dir/openclaw.log"
state_path="$state_dir/state.json"
mock_pid=""
connector_pid=""
use_existing_openclaw="${VIFU_E2E_USE_EXISTING_OPENCLAW:-0}"

compose() {
  docker compose --env-file "$env_file" -f "$compose_file" "$@"
}

cleanup_processes() {
  if [ -n "$connector_pid" ]; then kill "$connector_pid" 2>/dev/null || true; fi
  if [ -n "$mock_pid" ]; then kill "$mock_pid" 2>/dev/null || true; fi
}

on_failure() {
  status=$?
  if [ "$status" -ne 0 ]; then
    printf '%s\n' "--- connector log ---"
    tail -n 100 "$connector_log" 2>/dev/null || true
    printf '%s\n' "--- OpenClaw mock log ---"
    tail -n 100 "$mock_log" 2>/dev/null || true
    compose logs --no-color --tail=100 backend dashboard postgres || true
  fi
  cleanup_processes
  exit "$status"
}
trap on_failure EXIT INT TERM

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
VIFU_CONNECTOR_TOKEN="${VIFU_E2E_CONNECTOR_TOKEN:-$VIFU_CONNECTOR_TOKEN}" \
cargo run -p vifu -- \
  --openclaw-url "http://127.0.0.1:$openclaw_port" \
  --server-url "${VIFU_E2E_API_URL:-http://127.0.0.1:6790}" \
  >"$connector_log" 2>&1 &
connector_pid=$!

attempt=0
until curl --fail --silent \
  -H "Authorization: Bearer ${VIFU_E2E_ADMIN_KEY:-$VIFU_ADMIN_KEY}" \
  "${VIFU_E2E_API_URL:-http://127.0.0.1:6790}/v1/connections" \
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
  "${VIFU_E2E_API_URL:-http://127.0.0.1:6790}/v1/connections" \
  | grep -q '"status":"connected"'; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 60 ]; then exit 1; fi
  sleep 1
done

VIFU_E2E_STATE_PATH="$state_path" node scripts/test-self-hosted-e2e.mjs verify
VIFU_E2E_STATE_PATH="$state_path" node scripts/test-self-hosted-e2e.mjs cleanup

cleanup_processes
connector_pid=""
mock_pid=""
trap - EXIT INT TERM
printf '%s\n' "Self-hosted connector, persistence, and concurrency E2E passed."
