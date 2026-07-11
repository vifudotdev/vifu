#!/bin/sh
set -eu

if [ -f .env.local ]; then
  set -a
  . ./.env.local
  set +a
fi

exec cargo run -p vifu-server
