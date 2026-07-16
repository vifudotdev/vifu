#!/bin/sh
set -eu

env_file="${1:-.env}"
if [ -e "$env_file" ]; then
  printf '%s\n' "$env_file already exists; refusing to overwrite it." >&2
  exit 1
fi
if ! command -v openssl >/dev/null 2>&1; then
  printf '%s\n' "openssl is required to generate self-hosted secrets." >&2
  exit 1
fi

umask 077
admin_key="$(openssl rand -hex 32)"
agent_gateway_token="$(openssl rand -hex 32)"
api_key_pepper="$(openssl rand -hex 32)"
provider_secret_key="$(openssl rand -hex 32)"
postgres_password="$(openssl rand -hex 24)"
database_url_prefix="postgres://vifu:"
database_url_suffix="@postgres:5432/vifu"

{
  printf '%s\n' "VIFU_DEPLOYMENT_MODE=self-hosted"
  printf '%s\n' "VIFU_AUTH_MODE=local-password"
  printf '%s\n' "VIFU_AUTH_PASSWORD_ENABLED=true"
  printf '%s\n' "VIFU_SIGNUP_ENABLED=true"
  printf '%s\n' "AUTH_DISABLE_USERNAME_PASSWORD=false"
  printf '%s\n' "AUTH_DISABLE_SIGNUP=false"
  printf '%s\n' "VIFU_ADMIN_KEY=$admin_key"
  printf '%s\n' "VIFU_AGENT_GATEWAY_TOKEN=$agent_gateway_token"
  printf '%s\n' "VIFU_API_KEY_PEPPER=$api_key_pepper"
  printf '%s\n' "VIFU_PROVIDER_SECRET_KEY=$provider_secret_key"
  printf '%s\n' "POSTGRES_DB=vifu"
  printf '%s\n' "POSTGRES_USER=vifu"
  printf '%s\n' "POSTGRES_PASSWORD=$postgres_password"
  printf '%s%s%s\n' "DATABASE_URL=$database_url_prefix" "$postgres_password" "$database_url_suffix"
  printf '%s\n' "VIFU_BIND_HOST=127.0.0.1"
} > "$env_file"

printf '%s\n' "Created $env_file with independent self-hosted secrets."
