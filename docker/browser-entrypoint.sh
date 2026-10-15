#!/bin/sh
# Keep Chromium and its private network proxy in the same supervised container.
set -eu
chromium --headless=new --no-sandbox --disable-dev-shm-usage \
  --remote-debugging-port=9223 --remote-allow-origins='*' \
  --window-size=1280,800 --user-data-dir=/tmp/kwnt-chromium about:blank &
chrome_pid=$!
socat TCP-LISTEN:9222,fork,reuseaddr TCP:127.0.0.1:9223 &
proxy_pid=$!
trap 'kill "$chrome_pid" "$proxy_pid" 2>/dev/null || true' EXIT INT TERM
wait "$chrome_pid"
