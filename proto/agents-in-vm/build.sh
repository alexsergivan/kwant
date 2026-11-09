#!/usr/bin/env bash
# Builds kwnt-proto-lin-agent:spike from the host's installed CLI binaries.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
CTX=$(mktemp -d /tmp/kwnt-proto-lin-ctx.XXXX)
trap 'rm -rf "$CTX"' EXIT
cp "$(readlink -f /home/maus/.openmausbot/tools/npm/bin/claude)" "$CTX/claude"
cp /home/maus/.local/lib/node_modules/@openai/codex/node_modules/@openai/codex-linux-arm64/vendor/aarch64-unknown-linux-musl/bin/codex "$CTX/codex"
cp "$(readlink -f /home/maus/.grok/bin/grok)" "$CTX/grok"
cp "$HERE/Dockerfile.agent" "$HERE/entry.sh" "$CTX/"
docker build -q -t kwnt-proto-lin-agent:spike -f "$CTX/Dockerfile.agent" "$CTX"
