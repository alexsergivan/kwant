#!/bin/sh
# Keep Chromium and its private network proxy in the same supervised container.
set -eu
# Chromium keeps its own user agent: a --user-agent flag empties the client hints a real Chrome
# fills, and Google refuses sign-in on that. kwant takes the headless token out over DevTools
# with the browser's own client hints (src/sandbox/identity.rs). AutomationControlled off keeps
# navigator.webdriver false while the debugging port is open.
set -- --headless=new --no-sandbox --disable-dev-shm-usage \
  --disable-blink-features=AutomationControlled \
  --remote-debugging-port=9223 --remote-allow-origins='*' \
  --window-size=1280,800 --user-data-dir=/tmp/kwnt-chromium
# Optional proxy for the browser's traffic (KWNT_BROWSER_PROXY=socks5://host:port or http://host:port).
# Kwant's own services and private ranges skip it; KWNT_BROWSER_PROXY_BYPASS adds hosts.
# Never print the value: only whether it was taken.
if [ -n "${KWNT_BROWSER_PROXY:-}" ]; then
  case "$KWNT_BROWSER_PROXY" in
    socks5://*|socks5h://*|http://*|https://*) ;;
    *) echo "KWNT_BROWSER_PROXY: expected socks5://host:port or http://host:port" >&2; exit 2 ;;
  esac
  case "$KWNT_BROWSER_PROXY" in
    *@*|*' '*|*';'*|*'"'*) echo "KWNT_BROWSER_PROXY: credentials and spaces are not supported" >&2; exit 2 ;;
  esac
  bypass="laya;kwnt;browser;localhost;127.0.0.0/8;10.0.0.0/8;172.16.0.0/12;192.168.0.0/16;169.254.0.0/16"
  extra=$(printf '%s' "${KWNT_BROWSER_PROXY_BYPASS:-}" | tr ',' ';')
  [ -z "$extra" ] || bypass="$bypass;$extra"
  set -- "$@" "--proxy-server=$KWNT_BROWSER_PROXY" "--proxy-bypass-list=$bypass"
  echo "browser proxy: on (${KWNT_BROWSER_PROXY%%://*})"
fi
chromium "$@" about:blank &
chrome_pid=$!
socat TCP-LISTEN:9222,fork,reuseaddr TCP:127.0.0.1:9223 &
proxy_pid=$!
trap 'kill "$chrome_pid" "$proxy_pid" 2>/dev/null || true' EXIT INT TERM
wait "$chrome_pid"
