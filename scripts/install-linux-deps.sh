#!/bin/sh
# Installs what kwant needs on a Debian/Ubuntu server without Docker: Chromium (the bots'
# browser), Python 3 with venv (kwnt serve sets Laya up in it) and, if asked, the AI CLIs for
# the current user. Prints every command before running it. Safe to run again.
#
#   sh scripts/install-linux-deps.sh                      # browser + Python
#   sh scripts/install-linux-deps.sh --clis claude,codex  # also the CLIs (claude, codex, grok)
#
# Then: kwnt doctor
set -eu

clis=""
while [ $# -gt 0 ]; do
  case "$1" in
    --clis) clis="${2:-}"; shift 2 ;;
    --clis=*) clis="${1#--clis=}"; shift ;;
    -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

run() { echo "+ $*"; "$@"; }
sudo=""; [ "$(id -u)" -ne 0 ] && sudo="sudo"
has() { command -v "$1" >/dev/null 2>&1; }

if ! has apt-get; then
  echo "This script is for Debian/Ubuntu (apt). Elsewhere install Chromium and Python 3 yourself, then run kwnt doctor." >&2
  exit 1
fi

. /etc/os-release 2>/dev/null || true
run $sudo apt-get update
run $sudo apt-get install -y --no-install-recommends ca-certificates curl python3 python3-venv fonts-liberation
if has chromium || has chromium-browser || has google-chrome || has google-chrome-stable; then
  echo "Chrome/Chromium already installed"
elif [ "${ID:-}" = ubuntu ]; then
  # Ubuntu's chromium apt package is only a wrapper for the snap
  run $sudo snap install chromium
else
  run $sudo apt-get install -y --no-install-recommends chromium
fi

for c in $(echo "$clis" | tr ',' ' '); do
  case "$c" in
    claude) has claude && echo "claude already installed" || { echo "+ curl -fsSL https://claude.ai/install.sh | bash"; curl -fsSL https://claude.ai/install.sh | bash; } ;;
    grok) has grok && echo "grok already installed" || { echo "+ curl -fsSL https://x.ai/cli/install.sh | bash"; curl -fsSL https://x.ai/cli/install.sh | bash; } ;;
    codex)
      if has codex; then echo "codex already installed"
      elif has npm; then run npm install -g @openai/codex
      else echo "codex needs Node.js 18+ and npm: install them (https://nodejs.org), then: npm install -g @openai/codex" >&2
      fi ;;
    *) echo "unknown CLI '$c' (use claude, codex, grok)" >&2; exit 2 ;;
  esac
done

echo
echo "Done. Sign the CLIs in (claude auth login, codex login --device-auth, grok login --device-auth)"
echo "or add an API key in the app, then run: kwnt doctor"
