#!/bin/sh
set -eu

if [ -f .env.local ]; then
  set -a
  . ./.env.local
  set +a
elif [ -f .env ]; then
  set -a
  . ./.env
  set +a
fi

exec cargo run -p vifu
