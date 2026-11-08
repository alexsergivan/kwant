#!/usr/bin/env bash
# Gate 0: the spike against the REAL api.anthropic.com, API-key mode, Haiku, three short turns
# (a normal turn, a --resume turn, one Bash tool call; about six calls in all).
#   proto/agents-in-vm/gate0.sh [key file]     (default /tmp/ada-claude-key/anthropic_api_key)
# The key is read once from that file into the broker's 0600 creds file, never printed or
# logged; every copy this script makes is shredded on exit. Writes evidence/real-upstream.txt:
# every (method, path, status) the CLI sent, denied ones included, with hashes only.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); RUN="$HERE/run-gate0"; KEYFILE=${1:-/tmp/ada-claude-key/anthropic_api_key}
C=kwnt-proto-lin-gate0
[ -r "$KEYFILE" ] || { echo "no key at $KEYFILE" >&2; exit 2; }
cleanup() {
  docker rm -f "$C" >/dev/null 2>&1
  [ -f "$RUN/host/broker.pid" ] && kill "$(cat "$RUN/host/broker.pid")" 2>/dev/null
  [ -f "$RUN/host/creds.json" ] && shred -u "$RUN/host/creds.json" 2>/dev/null; rm -f "$RUN/host/creds.json"
}
trap cleanup EXIT
rm -rf "$RUN"; mkdir -p "$RUN"/{host,sock,task,home}; chmod 700 "$RUN/host"
docker image inspect kwnt-proto-lin-agent:spike >/dev/null 2>&1 || "$HERE/build.sh"
# The broker's creds file: the only copy of the key this script makes (0600, shredded on exit).
( umask 077; python3 -I - "$KEYFILE" "$RUN/host/creds.json" <<'P'
import json, sys
key = open(sys.argv[1]).read().strip()
json.dump({"anthropic": {"mode": "api_key", "api_key": key}}, open(sys.argv[2], "w"))
P
)
KEYSHA=$(python3 -I -c 'import hashlib,json,sys;print(hashlib.sha256(json.load(open(sys.argv[1]))["anthropic"]["api_key"].encode()).hexdigest()[:8])' "$RUN/host/creds.json")
nohup python3 "$HERE/broker.py" "$RUN/sock/broker.sock" "$RUN/host/creds.json" "$RUN/host/broker.log" https://api.anthropic.com \
  > "$RUN/host/broker.out" 2>&1 &
echo $! > "$RUN/host/broker.pid"
for _ in $(seq 50); do [ -S "$RUN/sock/broker.sock" ] && break; sleep 0.1; done
echo "gate0 test file" > "$RUN/task/README.txt"
docker run -d --name "$C" --label kwnt.proto=lin \
  --network none --user "$(id -u):$(id -g)" --cap-drop ALL --security-opt no-new-privileges \
  --read-only --tmpfs /tmp:rw,size=256m --cpus 2 --memory 1536m --pids-limit 512 \
  -e HOME=/home/agent \
  -v "$RUN/task:/workspace:rw" -v "$RUN/home:/home/agent:rw" -v "$RUN/sock:/run/kwnt:ro" \
  kwnt-proto-lin-agent:spike >/dev/null || exit 1
for _ in $(seq 50); do docker exec "$C" bash -c 'exec 3<>/dev/tcp/127.0.0.1/8080' 2>/dev/null && break; sleep 0.1; done
turn() { # turn <name> <prompt> [extra claude args...]
  local name=$1 prompt=$2; shift 2
  docker exec -i -w /workspace \
    -e HOME=/home/agent -e CLAUDE_CONFIG_DIR=/home/agent/.claude \
    -e ANTHROPIC_BASE_URL=http://127.0.0.1:8080/anthropic -e ANTHROPIC_API_KEY=sk-ant-api03-kwnt-dummy \
    -e CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 -e DISABLE_AUTOUPDATER=1 \
    -e HTTPS_PROXY=http://127.0.0.1:8080 -e HTTP_PROXY=http://127.0.0.1:8080 -e NO_PROXY=127.0.0.1,localhost \
    -e CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR=1 \
    "$C" claude --model haiku --permission-mode default --strict-mcp-config \
      --disallowedTools CronCreate,CronDelete,CronList,Task \
      --append-system-prompt "You are a kwant bot (Gate 0 check). Keep replies to one line." \
      --output-format stream-json --verbose --include-partial-messages "$@" -p "$prompt" \
      < /dev/null > "$RUN/$name.jsonl" 2> "$RUN/$name.err"
  local rc=$?
  echo "$(date -u +%H:%M:%S) $name rc=$rc result=$(jq -r 'select(.type=="result") | "\(.subtype) is_error=\(.is_error) turns=\(.num_turns)"' "$RUN/$name.jsonl" 2>/dev/null)"
  printf '%s\n' "$(date -u +%s.%N)" > "$RUN/host/mark-$name"
}
turn 1-normal "Reply with exactly: gate0 ok"
SID=$(jq -r 'select(.type=="system" and .subtype=="init") | .session_id' "$RUN/1-normal.jsonl" | head -1)
turn 2-resume "What exact words did you reply last time? One line." --resume "$SID"
turn 3-bash "Use the Bash tool to run: echo gate0-bash > probe.txt && cat probe.txt. Then reply with the output." --allowedTools Bash
BASH_OK=$(cat "$RUN/task/probe.txt" 2>/dev/null)
OWNER=$(stat -c '%u:%g' "$RUN/task/probe.txt" 2>/dev/null)
OUT="$HERE/evidence/real-upstream.txt"
python3 -I - "$RUN" "$KEYSHA" "$BASH_OK" "$OWNER" "$(id -u):$(id -g)" > "$OUT" <<'P'
import collections, glob, json, os, sys
run, keysha, bash_ok, owner, me = sys.argv[1:6]
recs = [json.loads(l) for l in open(f"{run}/host/broker.log")]
marks = sorted((float(open(m).read()), os.path.basename(m)[5:]) for m in glob.glob(f"{run}/host/mark-*"))
def turn_of(t):
    return next((n for end, n in marks if t <= end), "after")
print("# Gate 0: Claude Code CLI against the real api.anthropic.com through the spike broker")
print("# CLI 2.1.316 (native, aarch64) in --network none; ANTHROPIC_BASE_URL=http://127.0.0.1:8080/anthropic;")
print("# HTTPS_PROXY/HTTP_PROXY also point at the broker so calls to any other host are logged (CONNECT) and denied.")
print("# CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 DISABLE_AUTOUPDATER=1; --model haiku; API-key mode.")
print(f"# real key sha8={keysha} (injected by the broker); container credential = a dummy")
print("# columns: turn  method  path(long/id-like parts hashed)  status  broker-decision  sent-cred-sha8  injected-cred-sha8")
for r in recs:
    print("\t".join(map(str, [turn_of(r["t"]), r["method"], r["path"], r.get("status", "-"), r["decision"],
                              r.get("sent_cred_sha", "-"), r.get("real_cred_sha", "-")])))
print()
print("# summary (method path status decision -> count)")
for k, n in sorted(collections.Counter((r["method"], r["path"], r.get("status", "-"), r["decision"]) for r in recs).items()):
    print(f"#   {n:3d}  {' '.join(map(str, k))}")
for n in ("1-normal", "2-resume", "3-bash"):
    res = [json.loads(l) for l in open(f"{run}/{n}.jsonl") if l.startswith("{")]
    fin = next((x for x in res if x.get("type") == "result"), {})
    print(f"# turn {n}: subtype={fin.get('subtype')} is_error={fin.get('is_error')} num_turns={fin.get('num_turns')} "
          f"result_sha8={__import__('hashlib').sha256(str(fin.get('result')).encode()).hexdigest()[:8]} result_len={len(str(fin.get('result') or ''))}")
print(f"# bash tool wrote probe.txt: {bash_ok!r}; file owner uid:gid {owner} (host user {me})")
print(f"# calls forwarded upstream: {sum(r['decision']=='forward' for r in recs)}; denied: {sum(r['decision'].startswith('deny') for r in recs)}")
P
echo "--- $OUT"; cat "$OUT"
for n in 1-normal 2-resume 3-bash; do echo "--- $n result"; jq -r 'select(.type=="result") | .result' "$RUN/$n.jsonl" | head -3; done
grep -l 'sk-ant-api03-[A-Za-z0-9_-]\{30,\}' "$RUN"/*.jsonl "$RUN"/*.err "$RUN/host/broker.log" "$OUT" 2>/dev/null && echo "WARNING: key-shaped string found" || echo "no key-shaped string in any output"
