#!/bin/sh
set -eu

if [ -f .env.local ]; then
  set -a
  . ./.env.local
  set +a
fi

exec bun run --cwd npm-packages/dashboard dev
