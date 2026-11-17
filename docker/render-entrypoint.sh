#!/bin/sh
# Hand the disk to kwnt (uid 10001) once, then drop root for good and start as the image does.
set -eu
data="${KWNT_DATA_DIR:-/data}"
if [ "$(id -u)" = 0 ]; then
  mkdir -p "$data"
  [ "$(stat -c %u "$data")" = 10001 ] || chown -R 10001:10001 "$data"
  exec setpriv --reuid=10001 --regid=10001 --init-groups -- /usr/local/bin/kwnt-entrypoint "$@"
fi
exec /usr/local/bin/kwnt-entrypoint "$@"
