#!/bin/sh
set -eu

state_dir="$(mktemp -d "${TMPDIR:-/tmp}/vifu-oidc-e2e.XXXXXX")"
postgres_name="vifu-oidc-e2e-$$"
postgres_pid=""
oidc_pid=""
server_pid=""
dashboard_pid=""

free_port() {
  node -e 'const net=require("node:net");const server=net.createServer();server.listen(0,"127.0.0.1",()=>{console.log(server.address().port);server.close();});'
}

postgres_port="$(free_port)"
oidc_port="$(free_port)"
server_port="$(free_port)"
dashboard_port="$(free_port)"
postgres_password="$(openssl rand -hex 24)"
admin_key="$(openssl rand -hex 32)"
agent_gateway_bootstrap_token="$(openssl rand -hex 32)"
api_key_pepper="$(openssl rand -hex 32)"
provider_secret_key="$(openssl rand -hex 32)"
database_url="$(printf '%s%s%s%s%s%s%s\n' 'postgres://' 'vifu' ':' "$postgres_password" '@127.0.0.1:' "$postgres_port" '/vifu')"

cleanup() {
  status=$?
  for pid in "$dashboard_pid" "$server_pid" "$oidc_pid"; do
    if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; fi
  done
  docker rm -f "$postgres_name" >/dev/null 2>&1 || true
  if [ "$status" -ne 0 ]; then
    printf '%s\n' "--- OIDC provider ---"
    tail -n 80 "$state_dir/oidc.log" 2>/dev/null || true
    printf '%s\n' "--- Vifu server ---"
    tail -n 80 "$state_dir/server.log" 2>/dev/null || true
    printf '%s\n' "--- Dashboard ---"
    tail -n 80 "$state_dir/dashboard.log" 2>/dev/null || true
  fi
  rm -rf -- "$state_dir"
  exit "$status"
}
trap cleanup EXIT INT TERM

docker run -d --rm \
  --name "$postgres_name" \
  -e POSTGRES_DB=vifu \
  -e POSTGRES_USER=vifu \
  -e "POSTGRES_PASSWORD=$postgres_password" \
  -p "127.0.0.1:$postgres_port:5432" \
  postgres:17-bookworm > /dev/null

attempt=0
until docker exec "$postgres_name" pg_isready -U vifu -d vifu >/dev/null 2>&1; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 30 ]; then exit 1; fi
  sleep 1
done

VIFU_OIDC_TEST_PORT="$oidc_port" node scripts/mock-oidc-provider.mjs >"$state_dir/oidc.log" 2>&1 &
oidc_pid=$!

runtime_home="$state_dir/runtime-home"
mkdir -p "$runtime_home/.vifu"
printf '{\n  "version": 1,\n  "server": { "listen": "127.0.0.1:%s" }\n}\n' "$server_port" > "$runtime_home/.vifu/config.json"
cargo build -p vifu

VIFU_DEPLOYMENT_MODE=self-hosted \
DATABASE_URL="$database_url" \
VIFU_ADMIN_KEY="$admin_key" \
VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN="$agent_gateway_bootstrap_token" \
VIFU_API_KEY_PEPPER="$api_key_pepper" \
VIFU_PROVIDER_SECRET_KEY="$provider_secret_key" \
HOME="$runtime_home" \
RUST_LOG=tower_http=error \
target/debug/vifu >"$state_dir/server.log" 2>&1 &
server_pid=$!

attempt=0
until curl --fail --silent "http://127.0.0.1:$oidc_port/health" >/dev/null \
  && curl --fail --silent "http://127.0.0.1:$server_port/health" >/dev/null; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 90 ]; then exit 1; fi
  sleep 1
done

(
  cd npm-packages/dashboard
  VIFU_E2E=1 \
  VIFU_DEPLOYMENT_MODE=self-hosted \
  VIFU_AUTH_MODE=local-password \
  VIFU_AUTH_PASSWORD_ENABLED=true \
  AUTH_ENABLE_OIDC=true \
  VIFU_API_BASE_URL="http://127.0.0.1:$server_port" \
  VIFU_DASHBOARD_URL="http://localhost:$dashboard_port" \
  DATABASE_URL="$database_url" \
  VIFU_ADMIN_KEY="$admin_key" \
  VIFU_AUTH_OIDC_ISSUER="http://127.0.0.1:$oidc_port" \
  VIFU_AUTH_OIDC_CLIENT_ID=vifu-oidc-e2e \
  VIFU_AUTH_OIDC_CLIENT_SECRET=vifu-oidc-e2e-secret \
  VIFU_AUTH_OIDC_REDIRECT_URL="http://localhost:$dashboard_port/api/auth/oidc/oidc/callback" \
  VIFU_AUTH_OIDC_NAME="Continue with Test Identity" \
  VIFU_AUTH_OIDC_BOOTSTRAP_EMAIL=oidc-admin@example.com \
  node_modules/.bin/next dev -H 127.0.0.1 -p "$dashboard_port"
) >"$state_dir/dashboard.log" 2>&1 &
dashboard_pid=$!

attempt=0
until curl --fail --silent "http://127.0.0.1:$dashboard_port/login" >/dev/null; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 90 ]; then exit 1; fi
  sleep 1
done

VIFU_OIDC_E2E_DASHBOARD_URL="http://localhost:$dashboard_port" \
npx playwright test --config playwright.oidc.config.ts
