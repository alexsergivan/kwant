#!/usr/bin/env bash
# Removes every kwnt-proto-lin-* container and network and stops the mock and broker.
HERE=$(cd "$(dirname "$0")" && pwd)
for c in $(docker ps -aq --filter name=^kwnt-proto-lin-); do docker rm -f "$c" >/dev/null; done
for n in $(docker network ls -q --filter name=^kwnt-proto-lin-); do docker network rm "$n" >/dev/null; done
for p in "$HERE"/run/host/*.pid; do [ -f "$p" ] && kill "$(cat "$p")" 2>/dev/null; rm -f "$p"; done
echo "down: $(docker ps -a --filter name=^kwnt-proto-lin- -q | wc -l) containers, $(docker network ls --filter name=^kwnt-proto-lin- -q | wc -l) networks left"
