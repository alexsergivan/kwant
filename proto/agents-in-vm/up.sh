#!/usr/bin/env bash
# Starts the spike: mock upstream (127.0.0.1:19471), broker (Unix socket), and the
# --network none agent container kwnt-proto-lin-agent. Fresh canary tokens every time.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
RUN="$HERE/run"
PORT=19471
"$HERE/down.sh" >/dev/null 2>&1 || true
rm -rf "$RUN"; mkdir -p "$RUN"/{host,sock,task,home} "$RUN/host/signin-home/.claude" "$RUN/host/signin-home/.codex"
chmod 700 "$RUN/host"
docker image inspect kwnt-proto-lin-agent:spike >/dev/null 2>&1 || "$HERE/build.sh"
r() { head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n'; }
KEY="sk-ant-api03-REALCANARY-$(r)"; ACC="sk-ant-oat01-REALCANARY-$(r)"; REF="sk-ant-ort01-REALCANARY-$(r)"
OAI="sk-proj-REALCANARY-$(r)"; CGPT="eyREALCANARY.$(r)"; XAI="xai-REALCANARY-$(r)"; GW="kwnt-gw-$(r)"
EXP=$(( ($(date +%s) + 3600) * 1000 ))
umask 077
# The host's real sign-in home (what ~/.claude/.credentials.json looks like on Linux).
cat > "$RUN/host/signin-home/.claude/.credentials.json" <<J
{"claudeAiOauth":{"accessToken":"$ACC","refreshToken":"$REF","expiresAt":$EXP,"scopes":["user:inference"]}}
J
echo "{\"OPENAI_API_KEY\":\"$OAI\",\"tokens\":{\"access_token\":\"$CGPT\",\"account_id\":\"acct-canary\"}}" > "$RUN/host/signin-home/.codex/auth.json"
# What the broker reads (in kwnt: the profile's sign-in home / the keys table).
cat > "$RUN/host/creds.json" <<J
{"anthropic":{"mode":"api_key","api_key":"$KEY","oauth":{"accessToken":"$ACC","refreshToken":"$REF","expiresAt":$EXP}},
 "openai":{"api_key":"$OAI"},"chatgpt":{"access_token":"$CGPT","account_id":"acct-canary"},"xai":{"api_key":"$XAI"}}
J
echo "{\"api_key\":\"$KEY\",\"access_token\":\"$ACC\",\"refresh_token\":\"$REF\",\"gateway_token\":\"$GW\",\"openai_key\":\"$OAI\",\"chatgpt_access\":\"$CGPT\"}" > "$RUN/host/upstream_state.json"
umask 022
# Per-turn MCP config, as mcp::cli_config writes it, pointed at the forwarder.
echo "{\"mcpServers\":{\"kwnt\":{\"type\":\"http\",\"url\":\"http://127.0.0.1:8080/mcp\",\"headers\":{\"Authorization\":\"Bearer $GW\"}}}}" > "$RUN/sock/mcp.json"
echo "hello from the task folder" > "$RUN/task/README.txt"
nohup python3 "$HERE/mock_upstream.py" $PORT "$RUN/host/upstream_state.json" "$RUN/host/upstream.log" > "$RUN/host/mock.out" 2>&1 &
echo $! > "$RUN/host/mock.pid"
nohup python3 "$HERE/broker.py" "$RUN/sock/broker.sock" "$RUN/host/creds.json" "$RUN/host/broker.log" $PORT > "$RUN/host/broker.out" 2>&1 &
echo $! > "$RUN/host/broker.pid"
for _ in $(seq 50); do [ -S "$RUN/sock/broker.sock" ] && break; sleep 0.1; done
docker run -d --name kwnt-proto-lin-agent --label kwnt.proto=lin \
  --network none --user "$(id -u):$(id -g)" --cap-drop ALL --security-opt no-new-privileges \
  --read-only --tmpfs /tmp:rw,size=256m --cpus 2 --memory 1536m --pids-limit 512 \
  -e HOME=/home/agent \
  -v "$RUN/task:/workspace:rw" -v "$RUN/home:/home/agent:rw" -v "$RUN/sock:/run/kwnt:ro" \
  kwnt-proto-lin-agent:spike >/dev/null
for _ in $(seq 50); do docker exec kwnt-proto-lin-agent bash -c 'exec 3<>/dev/tcp/127.0.0.1/8080' 2>/dev/null && break; sleep 0.1; done
echo "up: container kwnt-proto-lin-agent, broker $RUN/sock/broker.sock, mock 127.0.0.1:$PORT"
