#!/bin/bash
set -e

# Persistent user data directory for Chromium (saves cookies, local storage, logins, passwords)
CHROME_DATA_DIR="${CHROME_USER_DATA_DIR:-/workspace/.chromium_data}"
mkdir -p "$CHROME_DATA_DIR"

# Remote debugging port: --remote-debugging-port=9222
chromium \
  --headless=new \
  --remote-debugging-port=9223 \
  --remote-allow-origins="*" \
  --no-sandbox \
  --disable-gpu \
  --disable-dev-shm-usage \
  --disable-software-rasterizer \
  --window-size=1280,800 \
  --user-data-dir="$CHROME_DATA_DIR" \
  --password-store=basic \
  --enable-features=PasswordImport,AutofillServerCommunication \
  --enable-automation=false &

# Proxy incoming port 9222 from container network to local Chromium on 9223
socat TCP-LISTEN:9222,fork,reuseaddr TCP:127.0.0.1:9223 &

exec sleep infinity
