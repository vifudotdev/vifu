#!/bin/sh
set -eu

compose_file="docker-compose.yml"
state_dir="$(mktemp -d "${TMPDIR:-/tmp}/vifu-e2e.XXXXXX")"
managed_stack=0
compose_project=""
browser_dashboard_port=""
compose_override=""
docker_access_host="${VIFU_E2E_DOCKER_HOST:-127.0.0.1}"
docker_bind_host="${VIFU_E2E_DOCKER_BIND_HOST:-127.0.0.1}"

if [ -z "${VIFU_E2E_DOCKER_HOST:-}" ]; then
  docker_endpoint="$(docker context inspect --format '{{ .Endpoints.docker.Host }}' 2>/dev/null || true)"
  case "$docker_endpoint" in
    ssh://*)
      ssh_target="${docker_endpoint#ssh://}"
      ssh_target="${ssh_target%%/*}"
      resolved_host="$(ssh -G "$ssh_target" 2>/dev/null | awk '$1 == "hostname" { print $2; exit }')"
      docker_access_host="${resolved_host:-${ssh_target#*@}}"
      docker_access_host="${docker_access_host%%:*}"
      if [ -z "${VIFU_E2E_DOCKER_BIND_HOST:-}" ]; then
        docker_bind_host="$docker_access_host"
      fi
      ;;
  esac
fi

free_port() {
  node -e '
    const net = require("node:net");
    const excluded = new Set(
      process.argv.slice(1).map(Number).filter((port) => Number.isInteger(port) && port > 0),
    );
    const listen = () => {
      const server = net.createServer();
      server.listen(0, "127.0.0.1", () => {
        const port = server.address().port;
        server.close(() => {
          if (excluded.has(port)) listen();
          else console.log(port);
        });
      });
    };
    listen();
  ' "$@"
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

  previous_umask="$(umask)"
  umask 077
  admin_key="$(rand_hex 32)"
  agent_gateway_bootstrap_token="$(rand_hex 32)"
  api_key_pepper="$(rand_hex 32)"
  provider_secret_key="$(rand_hex 32)"
  postgres_password="$(rand_hex 24)"
  {
    printf '%s\n' "VIFU_DEPLOYMENT_MODE=self-hosted"
    printf '%s\n' "VIFU_ADMIN_KEY=$admin_key"
    printf '%s\n' "VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN=$agent_gateway_bootstrap_token"
    printf '%s\n' "VIFU_API_KEY_PEPPER=$api_key_pepper"
    printf '%s\n' "VIFU_PROVIDER_SECRET_KEY=$provider_secret_key"
    printf '%s\n' "VIFU_GUEST_BOOTSTRAP_ENABLED=false"
    printf '%s\n' "POSTGRES_DB=vifu"
    printf '%s\n' "POSTGRES_USER=vifu"
    printf '%s\n' "POSTGRES_PASSWORD=$postgres_password"
    printf '%s%s%s\n' "DATABASE_URL=postgres://vifu:" "$postgres_password" "@postgres:5432/vifu"
    printf '%s\n' "VIFU_BIND_HOST=$docker_bind_host"
  } > "$target"
  umask "$previous_umask"
}

if [ -n "${VIFU_E2E_ENV_FILE:-}" ]; then
  env_file="$VIFU_E2E_ENV_FILE"
else
  env_file="$state_dir/.env"
  write_e2e_env "$env_file"
  server_port="$(free_port)"
  dashboard_port="$(free_port "$server_port")"
  browser_dashboard_port="$dashboard_port"
  {
    printf '%s\n' "VIFU_SERVER_PORT=$server_port"
    printf '%s\n' "VIFU_DASHBOARD_PORT=$dashboard_port"
    printf '%s\n' "POSTGRES_PORT=0"
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
  openclaw_port="$(free_port "${server_port:-}" "${dashboard_port:-}")"
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
    "${VIFU_E2E_API_URL:-http://$docker_access_host:6790}/v1/agent-gateways" \
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
    compose logs --no-color --tail=100 agent-gateway pairing-agent-gateway backend dashboard postgres openclaw-mock runtime-state || true
  fi
  cleanup_processes
  if [ "$managed_stack" = "1" ]; then compose down --volumes --remove-orphans --rmi local >/dev/null 2>&1 || true; fi
  rm -rf -- "$state_dir"
  exit "$status"
}
trap on_failure EXIT INT TERM

if [ "$use_existing_openclaw" != "1" ] && [ "$managed_stack" != "1" ]; then
  openclaw_mock_host="127.0.0.1"
  OPENCLAW_MOCK_HOST="$openclaw_mock_host" \
  OPENCLAW_MOCK_PORT="$openclaw_port" \
  OPENCLAW_MOCK_TOKEN="$openclaw_provider_token" \
  node scripts/mock-openclaw.mjs >"$mock_log" 2>&1 &
  mock_pid=$!
fi

if [ "$use_existing_openclaw" = "1" ] || [ "$managed_stack" != "1" ]; then
  openclaw_health_host="127.0.0.1"
  if [ "$managed_stack" = "1" ]; then
    openclaw_health_host="$docker_access_host"
  fi
  attempt=0
  until curl --fail --silent "http://$openclaw_health_host:$openclaw_port/health" >/dev/null; do
    if [ -n "$mock_pid" ] && ! kill -0 "$mock_pid" 2>/dev/null; then
      cat "$mock_log" >&2
      exit 1
    fi
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 30 ]; then exit 1; fi
    sleep 1
  done
fi

gateway_root="$state_dir/gateway-home"
gateway_home="$gateway_root/.vifu"
mkdir -p "$gateway_home"
chmod 0711 "$state_dir" "$gateway_root"
chmod 0777 "$gateway_home"
provider_url="http://127.0.0.1:$openclaw_port"
if [ "$managed_stack" = "1" ]; then
  if [ "$use_existing_openclaw" = "1" ]; then
    provider_url="http://host.docker.internal:$openclaw_port"
  else
    provider_url="http://openclaw-mock:18789"
  fi
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
chmod 0644 "$gateway_home/providers.json"

if [ "$managed_stack" != "1" ]; then
  printf '{\n  "version": 1,\n  "gateway": { "serverUrl": "%s" }\n}\n' \
    "$(json_escape "${VIFU_E2E_API_URL:-http://127.0.0.1:6790}")" > "$gateway_home/config.json"
  chmod 0644 "$gateway_home/config.json"
fi

if [ "$managed_stack" = "1" ]; then
  compose_override="$state_dir/docker-compose.runtime-state.yml"
  providers_config="$(sed 's/^/      /' "$gateway_home/providers.json")"
  if [ "$use_existing_openclaw" = "1" ]; then
    cat > "$compose_override" <<EOF
configs:
  e2e_agent_providers:
    content: |
$providers_config
  e2e_pairing_gateway_config:
    content: |
      {
        "version": 1,
        "gateway": {
          "serverUrl": "http://backend:6790",
          "dashboardUrl": "http://$docker_access_host:$browser_dashboard_port",
          "guestBootstrap": false
        }
      }

services:
  runtime-state:
    image: node:22-bookworm-slim
    configs:
      - source: e2e_agent_providers
        target: /run/vifu/providers.json
        mode: 0444
    volumes:
      - vifu_server_state:/server-state
      - vifu_runtime_state:/gateway-state
    command: ["sh", "-c", "cp /run/vifu/providers.json /server-state/providers.json && cp /run/vifu/providers.json /gateway-state/providers.json && chmod 0644 /server-state/providers.json /gateway-state/providers.json"]
  backend:
    image: ${compose_project}-runtime:local
    depends_on:
      runtime-state:
        condition: service_completed_successfully
    environment:
      VIFU_REQUEST_TIMEOUT_MS: "500"
  agent-gateway:
    image: ${compose_project}-runtime:local
    configs:
      - source: e2e_agent_providers
        target: /home/vifu/.vifu/providers.json
        mode: 0444
  pairing-agent-gateway:
    image: ${compose_project}-runtime:local
    pull_policy: never
    depends_on:
      backend:
        condition: service_healthy
      runtime-state:
        condition: service_completed_successfully
    configs:
      - source: e2e_agent_providers
        target: /home/vifu/.vifu/providers.json
        mode: 0444
      - source: e2e_pairing_gateway_config
        target: /home/vifu/.vifu/config.json
        mode: 0444
    volumes:
      - vifu_pairing_runtime_state:/home/vifu/.vifu
    extra_hosts:
      - "host.docker.internal:host-gateway"

volumes:
  vifu_pairing_runtime_state:
EOF
  else
    cat > "$compose_override" <<EOF
configs:
  e2e_agent_providers:
    content: |
$providers_config
  e2e_pairing_gateway_config:
    content: |
      {
        "version": 1,
        "gateway": {
          "serverUrl": "http://backend:6790",
          "dashboardUrl": "http://$docker_access_host:$browser_dashboard_port",
          "guestBootstrap": false
        }
      }

services:
  runtime-state:
    image: node:22-bookworm-slim
    configs:
      - source: e2e_agent_providers
        target: /run/vifu/providers.json
        mode: 0444
    volumes:
      - vifu_server_state:/server-state
      - vifu_runtime_state:/gateway-state
    command: ["sh", "-c", "cp /run/vifu/providers.json /server-state/providers.json && cp /run/vifu/providers.json /gateway-state/providers.json && chmod 0644 /server-state/providers.json /gateway-state/providers.json"]
  backend:
    image: ${compose_project}-runtime:local
    depends_on:
      runtime-state:
        condition: service_completed_successfully
    environment:
      VIFU_REQUEST_TIMEOUT_MS: "500"
  agent-gateway:
    image: ${compose_project}-runtime:local
    depends_on:
      openclaw-mock:
        condition: service_healthy
    configs:
      - source: e2e_agent_providers
        target: /home/vifu/.vifu/providers.json
        mode: 0444
  pairing-agent-gateway:
    image: ${compose_project}-runtime:local
    pull_policy: never
    depends_on:
      backend:
        condition: service_healthy
      runtime-state:
        condition: service_completed_successfully
      openclaw-mock:
        condition: service_healthy
    configs:
      - source: e2e_agent_providers
        target: /home/vifu/.vifu/providers.json
        mode: 0444
      - source: e2e_pairing_gateway_config
        target: /home/vifu/.vifu/config.json
        mode: 0444
    volumes:
      - vifu_pairing_runtime_state:/home/vifu/.vifu
    extra_hosts:
      - "host.docker.internal:host-gateway"
  openclaw-mock:
    build:
      context: .
      dockerfile_inline: |
        FROM node:22-bookworm-slim
        WORKDIR /app
        COPY scripts/mock-openclaw.mjs /app/mock-openclaw.mjs
    working_dir: /app
    environment:
      OPENCLAW_MOCK_HOST: 0.0.0.0
      OPENCLAW_MOCK_PORT: "18789"
      OPENCLAW_MOCK_TOKEN: "$openclaw_provider_token"
    command: ["node", "/app/mock-openclaw.mjs"]
    ports:
      - "$docker_bind_host:$openclaw_port:18789"
    healthcheck:
      test: ["CMD", "node", "-e", "fetch('http://127.0.0.1:18789/health').then(r=>{if(!r.ok)process.exit(1)}).catch(()=>process.exit(1))"]
      interval: 1s
      timeout: 3s
      retries: 20

volumes:
  vifu_pairing_runtime_state:
EOF
  fi
  chmod 0600 "$compose_override"
  compose up -d --build --wait
  export VIFU_E2E_API_URL="http://$docker_access_host:$VIFU_SERVER_PORT"
  export VIFU_E2E_DASHBOARD_URL="http://$docker_access_host:$VIFU_DASHBOARD_PORT"
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
  VIFU_E2E_EXPECT_TIMEOUT="$managed_stack" \
  VIFU_E2E_STATE_PATH="$state_path" \
  node scripts/test-self-hosted-e2e.mjs setup
else
  VIFU_E2E_EXPECT_TIMEOUT="$managed_stack" \
  VIFU_E2E_OPENCLAW_MOCK_URL="http://$docker_access_host:$openclaw_port" \
  VIFU_E2E_STATE_PATH="$state_path" \
  node scripts/test-self-hosted-e2e.mjs setup
fi
if [ "$managed_stack" = "1" ]; then
  VIFU_E2E_STATE_PATH="$state_path" node scripts/test-self-hosted-e2e.mjs pairing
  compose restart pairing-agent-gateway
  VIFU_E2E_STATE_PATH="$state_path" node scripts/test-self-hosted-e2e.mjs pairing-restart
  VIFU_E2E_STATE_PATH="$state_path" node scripts/test-self-hosted-e2e.mjs pairing-revoke
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
VIFU_SELF_HOSTED_E2E_DASHBOARD_URL="http://$docker_access_host:$browser_dashboard_port" \
VIFU_SELF_HOSTED_E2E_ADMIN_KEY="$VIFU_ADMIN_KEY" \
npx playwright test --config playwright.self-hosted.config.ts
VIFU_E2E_STATE_PATH="$state_path" node scripts/test-self-hosted-e2e.mjs cleanup

cleanup_processes
agent_gateway_pid=""
mock_pid=""
if [ "$managed_stack" = "1" ]; then compose down --volumes --remove-orphans --rmi local >/dev/null; fi
rm -rf -- "$state_dir"
trap - EXIT INT TERM
printf '%s\n' "Self-hosted Agent Gateway, persistence, and concurrency E2E passed."
