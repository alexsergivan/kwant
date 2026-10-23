#!/bin/sh
# Keep Chromium and its private network proxy in the same supervised container.
set -eu
# Windowless Chromium tells sites it is "HeadlessChrome"; send the user agent the windowed
# Chromium of this version sends instead.
major=$(chromium --version | grep -oE '[0-9]+\.[0-9.]+' | head -n1 | cut -d. -f1)
agent="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${major}.0.0.0 Safari/537.36"
chromium --headless=new --no-sandbox --disable-dev-shm-usage --user-agent="$agent" \
  --remote-debugging-port=9223 --remote-allow-origins='*' \
  --window-size=1280,800 --user-data-dir=/tmp/kwnt-chromium about:blank &
chrome_pid=$!
socat TCP-LISTEN:9222,fork,reuseaddr TCP:127.0.0.1:9223 &
proxy_pid=$!
trap 'kill "$chrome_pid" "$proxy_pid" 2>/dev/null || true' EXIT INT TERM
wait "$chrome_pid"
