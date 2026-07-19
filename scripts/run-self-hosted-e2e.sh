#!/bin/sh
set -eu

compose_file="docker-compose.yml"
state_dir="$(mktemp -d "${TMPDIR:-/tmp}/vifu-e2e.XXXXXX")"
managed_stack=0
compose_project=""
browser_dashboard_port=""
compose_override=""

free_port() {
  node -e 'const net=require("node:net");const server=net.createServer();server.listen(0,"127.0.0.1",()=>{console.log(server.address().port);server.close();});'
}

json_escape() {
  node -e 'process.stdout.write(JSON.stringify(process.argv[1]).slice(1, -1));' "$1"
}

rand_hex() {
  openssl rand -hex "$1"
}

write_e2e_env() {
  target="$1"
  if ! command -v openssl >/dev/null 2>&1; then
    printf '%s\n' "openssl is required to generate E2E secrets." >&2
    exit 1
  fi

  umask 077
  admin_key="$(rand_hex 32)"
  agent_gateway_bootstrap_token="$(rand_hex 32)"
  api_key_pepper="$(rand_hex 32)"
  provider_secret_key="$(rand_hex 32)"
  postgres_password="$(rand_hex 24)"
  {
    printf '%s\n' "VIFU_DEPLOYMENT_MODE=self-hosted"
    printf '%s\n' "VIFU_AUTH_MODE=local-password"
    printf '%s\n' "VIFU_AUTH_PASSWORD_ENABLED=true"
    printf '%s\n' "VIFU_SIGNUP_ENABLED=true"
    printf '%s\n' "AUTH_DISABLE_USERNAME_PASSWORD=false"
    printf '%s\n' "AUTH_DISABLE_SIGNUP=false"
    printf '%s\n' "VIFU_ADMIN_KEY=$admin_key"
    printf '%s\n' "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN=$agent_gateway_bootstrap_token"
    printf '%s\n' "VIFU_API_KEY_PEPPER=$api_key_pepper"
    printf '%s\n' "VIFU_PROVIDER_SECRET_KEY=$provider_secret_key"
    printf '%s\n' "POSTGRES_DB=vifu"
    printf '%s\n' "POSTGRES_USER=vifu"
    printf '%s\n' "POSTGRES_PASSWORD=$postgres_password"
    printf '%s%s%s\n' "DATABASE_URL=postgres://vifu:" "$postgres_password" "@postgres:5432/vifu"
    printf '%s\n' "VIFU_BIND_HOST=127.0.0.1"
  } > "$target"
}

if [ -n "${VIFU_E2E_ENV_FILE:-}" ]; then
  env_file="$VIFU_E2E_ENV_FILE"
else
  env_file="$state_dir/.env"
  write_e2e_env "$env_file"
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
  printf '%s\n' "VIFU_ADMIN_KEY is missing. Set VIFU_E2E_ADMIN_KEY or use an E2E env file with an explicit admin key." >&2
  exit 1
fi
use_existing_openclaw="${VIFU_E2E_USE_EXISTING_OPENCLAW:-0}"
if [ -n "${VIFU_E2E_OPENCLAW_PORT:-}" ]; then
  openclaw_port="$VIFU_E2E_OPENCLAW_PORT"
elif [ "$use_existing_openclaw" = "1" ]; then
  openclaw_port="18789"
else
  openclaw_port="$(free_port)"
fi
agent_gateway_log="$state_dir/agent-gateway.log"
mock_log="$state_dir/openclaw.log"
state_path="$state_dir/state.json"
mock_pid=""
agent_gateway_pid=""
openclaw_provider_token="${OPENCLAW_GATEWAY_TOKEN:-}"
if [ "$use_existing_openclaw" != "1" ]; then
  openclaw_provider_token="${openclaw_provider_token:-$(rand_hex 32)}"
fi

compose() {
  if [ -n "$compose_project" ]; then
    if [ -n "$compose_override" ]; then
      docker compose -p "$compose_project" --env-file "$env_file" -f "$compose_file" -f "$compose_override" "$@"
    else
      docker compose -p "$compose_project" --env-file "$env_file" -f "$compose_file" "$@"
    fi
  else
    if [ -n "$compose_override" ]; then
      docker compose --env-file "$env_file" -f "$compose_file" -f "$compose_override" "$@"
    else
      docker compose --env-file "$env_file" -f "$compose_file" "$@"
    fi
  fi
}

agent_gateway_ready() {
  curl --fail --silent \
    -H "Authorization: Bearer ${VIFU_E2E_ADMIN_KEY:-$VIFU_ADMIN_KEY}" \
    "${VIFU_E2E_API_URL:-http://127.0.0.1:6790}/v1/agent-gateways" \
  | node -e '
      const payload = JSON.parse(require("node:fs").readFileSync(0, "utf8"));
      const ready = payload.agentGateways?.some((gateway) =>
        gateway.status === "connected"
          && gateway.agents?.some((agent) => agent.id === "guide-agent")
      );
      process.exit(ready ? 0 : 1);
    '
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
    compose logs --no-color --tail=100 agent-gateway backend dashboard postgres || true
  fi
  cleanup_processes
  if [ "$managed_stack" = "1" ]; then compose down --volumes --remove-orphans --rmi local >/dev/null 2>&1 || true; fi
  rm -rf -- "$state_dir"
  exit "$status"
}
trap on_failure EXIT INT TERM

if [ "$use_existing_openclaw" != "1" ]; then
  OPENCLAW_MOCK_PORT="$openclaw_port" \
  OPENCLAW_MOCK_TOKEN="$openclaw_provider_token" \
  node scripts/mock-openclaw.mjs >"$mock_log" 2>&1 &
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

gateway_root="$state_dir/gateway-home"
gateway_home="$gateway_root/.vifu"
mkdir -p "$gateway_home"
chmod 0777 "$gateway_home"
provider_url="http://127.0.0.1:$openclaw_port"
if [ "$managed_stack" = "1" ]; then
  provider_url="http://host.docker.internal:$openclaw_port"
fi
{
  printf '%s\n' '{'
  printf '%s\n' '  "providers": ['
  printf '%s\n' '    {'
  printf '%s\n' '      "key": "openclaw-e2e",'
  printf '%s\n' '      "type": "openclaw",'
  printf '%s' "      \"url\": \"$provider_url\""
  if [ -n "$openclaw_provider_token" ]; then
    printf '%s\n' ','
    printf '%s\n' "      \"auth\": { \"token\": \"$(json_escape "$openclaw_provider_token")\" }"
  else
    printf '%s\n' ''
  fi
  printf '%s\n' '    }'
  printf '%s\n' '  ]'
  printf '%s\n' '}'
} > "$gateway_home/providers.json"

if [ "$managed_stack" != "1" ]; then
  printf '{\n  "version": 1,\n  "gateway": { "serverUrl": "%s" }\n}\n' \
    "$(json_escape "${VIFU_E2E_API_URL:-http://127.0.0.1:6790}")" > "$gateway_home/config.json"
fi

if [ "$managed_stack" = "1" ]; then
  compose_override="$state_dir/docker-compose.runtime-state.yml"
  cat > "$compose_override" <<EOF
services:
  backend:
    volumes:
      - $gateway_home:/home/vifu/.vifu
  agent-gateway:
    volumes:
      - $gateway_home:/home/vifu/.vifu
EOF
  compose up -d --build --wait
  export VIFU_E2E_API_URL="http://127.0.0.1:$VIFU_SERVER_PORT"
  export VIFU_E2E_DASHBOARD_URL="http://127.0.0.1:$VIFU_DASHBOARD_PORT"
else
  cargo build -p vifu
  HOME="$gateway_root" \
  VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN="${VIFU_E2E_AGENT_GATEWAY_BOOTSTRAP_TOKEN:-$VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN}" \
  target/debug/vifu \
    >"$agent_gateway_log" 2>&1 &
  agent_gateway_pid=$!
fi

attempt=0
until agent_gateway_ready; do
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
until curl --fail --silent "${VIFU_E2E_DASHBOARD_URL:-http://127.0.0.1:6791}/project" >/dev/null; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 60 ]; then exit 1; fi
  sleep 1
done

attempt=0
until agent_gateway_ready; do
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
if [ "$managed_stack" = "1" ]; then compose down --volumes --remove-orphans --rmi local >/dev/null; fi
rm -rf -- "$state_dir"
trap - EXIT INT TERM
printf '%s\n' "Self-hosted Agent Gateway, persistence, and concurrency E2E passed."
