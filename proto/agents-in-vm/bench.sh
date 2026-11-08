#!/usr/bin/env bash
# One-turn latency against the mock: claude on the host vs in a warm container vs a cold one.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); RUN="$HERE/run"
"$HERE/up.sh" >/dev/null
KEY=$(jq -r .anthropic.api_key "$RUN/host/creds.json")
ms() { local s=$(date +%s%N); "$@" >/dev/null 2>&1; echo $(( ($(date +%s%N) - s) / 1000000 )); }
host_turn() {
  HOME_T=$(mktemp -d /tmp/kwnt-proto-lin-host.XXXX)
  (cd "$RUN/task" && HOME=$HOME_T CLAUDE_CONFIG_DIR=$HOME_T/.claude ANTHROPIC_BASE_URL=http://127.0.0.1:19471 \
   ANTHROPIC_API_KEY=$KEY CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 DISABLE_AUTOUPDATER=1 \
   timeout 60 claude --model claude-sonnet-4-5 --permission-mode default --strict-mcp-config \
   --output-format stream-json --verbose -p "bench" </dev/null)
  rm -rf "$HOME_T"
}
cold() {
  docker rm -f kwnt-proto-lin-agent >/dev/null
  docker run -d --name kwnt-proto-lin-agent --network none --user "$(id -u):$(id -g)" --cap-drop ALL \
    --security-opt no-new-privileges --read-only --tmpfs /tmp:rw,size=256m --cpus 2 --memory 1536m \
    -e HOME=/home/agent -v "$RUN/task:/workspace:rw" -v "$RUN/home:/home/agent:rw" -v "$RUN/sock:/run/kwnt:ro" \
    kwnt-proto-lin-agent:spike >/dev/null
  for _ in $(seq 100); do docker exec kwnt-proto-lin-agent bash -c "exec 3<>/dev/tcp/127.0.0.1/8080" 2>/dev/null && break; sleep 0.05; done
  "$HERE/turn.sh" api "bench"
}
for i in $(seq ${N:-5}); do echo "host $(ms host_turn) warm $(ms "$HERE/turn.sh" api bench) cold $(ms cold)"; done
