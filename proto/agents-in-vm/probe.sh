#!/usr/bin/env bash
# Runs INSIDE the agent container, as the bot would: tries the three attacks.
# Prints one line per attempt: "ok <what>" when the attack failed, "LEAK <what>" when it worked.
# HOST_PATHS: space-separated absolute host paths of sign-in files to try.
leaks=0
leak() { echo "LEAK $*"; leaks=$((leaks+1)); }
ok() { echo "ok   $*"; }
MARK="REAL""CANARY"   # every real credential the spike mints carries this marker; the container never gets one
echo "== 1. read the host sign-in files"
for p in $HOST_PATHS /root/.claude/.credentials.json /root/.codex/auth.json; do
  if out=$(cat "$p" 2>&1); then leak "read $p"; else ok "read $p -> ${out##*: }"; fi
done
found=$(find / -xdev \( -path /proc -o -path /sys -o -path /dev \) -prune -o \
  \( -name .credentials.json -o -name auth.json -o -name creds.json \) -print 2>/dev/null)
[ -z "$found" ] && ok "no sign-in file anywhere in the container" || leak "sign-in files present: $found"
echo "== 2. read the real token"
env | grep -q "$MARK" && leak "token in env" || ok "env holds no real token ($(env | grep -cE 'API_KEY|OAUTH_TOKEN') credential vars, all dummies)"
hit=""
for f in /proc/[0-9]*/environ /proc/[0-9]*/cmdline; do grep -aq "$MARK" "$f" 2>/dev/null && hit="$hit $f"; done
[ -z "$hit" ] && ok "no real token in any process env/cmdline ($(ls -d /proc/[0-9]* | wc -l) processes, incl. the running claude)" || leak "/proc:$hit"
files=$(grep -rla "$MARK" / --exclude-dir=proc --exclude-dir=sys --exclude-dir=dev 2>/dev/null | head)
[ -z "$files" ] && ok "no real token in any file (rootfs, /workspace, /home/agent, /run/kwnt, /tmp)" || leak "files: $files"
for path in /anthropic/v1/oauth/token /anthropic/v1/files /chatgpt/backend-api/accounts /admin; do
  code=$(curl -s -o /tmp/b -w '%{http_code}' -X POST "http://127.0.0.1:8080$path" -d '{}')
  grep -aq "$MARK" /tmp/b && leak "broker returned a token on $path" || ok "broker $path -> $code"
done
echo "== 3. reach an outside host"
for url in https://example.com http://1.1.1.1 http://172.17.0.1:22 http://host.docker.internal \
           http://127.0.0.1:8080/https://example.com; do
  code=$(curl -s -m 5 -o /dev/null -w '%{http_code}' "$url" 2>/dev/null); rc=$?
  if [ "$rc" = 0 ] && [ "${code:0:1}" = 2 ]; then leak "reached $url ($code)"; else ok "$url -> curl rc=$rc http=$code"; fi
done
echo "== 4. IPv6, UDP and DNS"
for url in "http://[2606:4700:4700::1111]" "http://[::ffff:1.1.1.1]"; do
  code=$(curl -s -m 5 -o /dev/null -w '%{http_code}' "$url" 2>/dev/null); rc=$?
  if [ "$rc" = 0 ] && [ "${code:0:1}" = 2 ]; then leak "reached $url ($code)"; else ok "$url -> curl rc=$rc http=$code"; fi
done
for dst in 1.1.1.1/53 2606:4700:4700::1111/53 172.17.0.1/53; do
  if timeout 5 bash -c "printf x > /dev/udp/$dst" 2>/dev/null; then leak "sent UDP to $dst"; else ok "UDP to $dst refused"; fi
done
if timeout 5 getent hosts example.com >/dev/null 2>&1; then leak "resolved example.com"; else ok "no DNS"; fi
echo "PROBE_LEAKS=$leaks"
