#!/bin/bash
set -e

# Launch Chromium in headless mode with remote debugging enabled
chromium \
  --headless=new \
  --remote-debugging-port=9222 \
  --remote-debugging-address=0.0.0.0 \
  --no-sandbox \
  --disable-gpu \
  --disable-dev-shm-usage \
  --disable-software-rasterizer \
  --window-size=1280,800 &

exec sleep infinity
