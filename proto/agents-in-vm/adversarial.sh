#!/usr/bin/env bash
# One command: proto/agents-in-vm/adversarial.sh   (KEEP=1 leaves the spike running)
# Starts the spike fresh, runs probe.sh inside the container twice (a plain shell, and the
# agent's own Bash tool driven by the scripted mock model through the broker), checks the
# positive control (the real token DID reach the upstream), then removes everything.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); RUN="$HERE/run"
"$HERE/up.sh" || exit 1
cp "$HERE/probe.sh" "$RUN/sock/probe.sh"
HP="$HOME/.claude/.credentials.json $HOME/.codex/auth.json $HOME/.grok/auth.json $RUN/host/signin-home/.claude/.credentials.json $RUN/host/creds.json $RUN/host/upstream_state.json"
fail=0
echo "### A. plain shell in the container (docker exec)"
docker exec -e HOST_PATHS="$HP" kwnt-proto-lin-agent bash /run/kwnt/probe.sh | tee "$RUN/probe-shell.txt"
grep -q '^PROBE_LEAKS=0$' "$RUN/probe-shell.txt" || fail=1
echo "### B. the agent's own Bash tool, mid-turn (claude in the container, real token at the broker)"
"$HERE/turn.sh" api "RUN: HOST_PATHS='$HP' bash /run/kwnt/probe.sh" > "$RUN/probe-agent.jsonl" 2>&1
python3 - "$RUN/host/upstream.log" > "$RUN/probe-agent.txt" <<'P'
import json, sys
for line in open(sys.argv[1]):
    r = json.loads(line)
    for t in r.get("tool_result", []):
        print(t.split("<system-reminder>")[0].rstrip())
P
cat "$RUN/probe-agent.txt"
grep -q '^PROBE_LEAKS=0$' "$RUN/probe-agent.txt" || { echo "agent probe did not report PROBE_LEAKS=0"; fail=1; }
echo "### C. positive control: the real key reached the upstream, the dummy never did"
real=$(jq -r 'select(.path|startswith("/v1/messages")) | .auth' "$RUN/host/upstream.log" | sort | uniq -c)
echo "$real"
echo "$real" | grep -q api_key_ok || { echo "real key never arrived upstream"; fail=1; }
echo "$real" | grep -q rejected && { echo "a dummy reached the upstream"; fail=1; }
grep -q 'kwnt-dummy' "$RUN/host/upstream.log" && { echo "dummy credential seen upstream"; fail=1; }
denied=$(jq -r 'select(.decision|startswith("deny")) | "\(.decision) \(.method) \(.path)"' "$RUN/host/broker.log")
echo "broker denials:"; echo "$denied" | sed 's/^/  /'
echo "### D. self-test: the probe does catch a planted token (expects leaks)"
docker exec -e HOST_PATHS="$HP" -e PLANTED="sk-ant-api03-REAL""CANARY-planted" kwnt-proto-lin-agent \
  sh -c 'echo "$PLANTED" > /tmp/planted; bash /run/kwnt/probe.sh' > "$RUN/probe-selftest.txt"
grep '^LEAK' "$RUN/probe-selftest.txt" | sed 's/^/  /'
grep -q '^PROBE_LEAKS=[1-9]' "$RUN/probe-selftest.txt" || { echo "self-test: probe missed a planted token"; fail=1; }
[ "${KEEP:-0}" = 1 ] || "$HERE/down.sh"
[ $fail = 0 ] && echo "RESULT: PASS (all three attacks failed in both paths)" || echo "RESULT: FAIL"
exit $fail
