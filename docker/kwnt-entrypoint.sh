#!/bin/sh
# Images before the non-root user left /data owned by root; say how to fix that instead of failing later.
set -eu
data="${KWNT_DATA_DIR:-/data}"
if [ "${1:-}" = kwnt ] && [ "${2:-}" = serve ] && ! [ -w "$data" ]; then
  echo "kwnt: $data isn't writable by uid $(id -u) (volumes made by older kwant images belong to root)." >&2
  echo "Fix it once with: docker compose run --rm -u root kwnt chown -R 10001:10001 $data" >&2
  exit 1
fi
exec "$@"
