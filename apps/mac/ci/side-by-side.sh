#!/usr/bin/env bash
# Side by side on a real desktop (next-075/side-by-side): This Mac plus one server, each in its
# own window, at once; then quit and reopen, and both come back from desktop-places.json's
# `open` with nothing typed. Also the move from the old desktop-target.json.
#
#   side-by-side.sh <out dir> <app binary>
#
# The server is a second kwant: the same app binary with --serve-headless on a free port, behind
# a small proxy that logs each request line and User-Agent, so the log shows the app's window
# (a WebKit User-Agent) loading the server's pages. macOS: run as is. Linux: run inside
# xvfb-run + dbus-run-session. Screenshots go to <out dir>; they never fail the run, the checks do.
set -uo pipefail
out=$1
app=$2
mkdir -p "$out"
work=$(mktemp -d)
mac=0
[ "$(uname)" = Darwin ] && mac=1
if [ $mac = 1 ]; then applog=mac-app.log; else applog=desktop-app.log; fi
failed=0
fail() { echo "::error::side by side: $*"; failed=1; }
note() { echo "side by side: $*"; }
free_port() { python3 -I -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])'; }
health() { curl -fsS --max-time 2 "http://127.0.0.1:$1/api/health" 2>/dev/null; }
wait_health() { for _ in $(seq 1 "$2"); do health "$1" > /dev/null && return 0; sleep 1; done; return 1; }

shot() {
  if [ $mac = 1 ]; then screencapture -x "$out/$1.png" || true
  else command -v import > /dev/null && import -window root "$out/$1.png" || true; fi
}

# Window titles, and on macOS the two windows moved apart so the screenshot shows both: GitHub's
# Mac screen is 1024x768, so two 480x680 windows at x=20 and x=520 fit without covering each other.
windows() {
  if [ $mac = 1 ]; then
    osascript > "$out/$1.txt" 2>&1 <<'OSA' || true
tell application "System Events"
  set p to first process whose bundle identifier is "si.kwant.mac"
  set ws to every window of p
  set i to 0
  repeat with w in ws
    try
      set position of w to {20 + i * 500, 40}
      set size of w to {480, 680}
    end try
    set i to i + 1
  end repeat
  return name of every window of p
end tell
OSA
  else
    xwininfo -root -tree 2> /dev/null | grep -o '"kwant[^"]*"' | sort -u > "$out/$1.txt" || true
  fi
  note "windows: $(tr '\n' ' ' < "$out/$1.txt")"
}

# Both windows are open: one titled This Mac (macOS) or This computer (Linux), one with the
# server's host:port. This Mac answers /api/health without a window, so the health check alone
# can't tell. macOS gives one comma-separated line; Linux one quoted title per line, plus GTK's
# title-less "kwant" leader window, which is ignored.
windows_check() {
  local want=${origin#http://}
  python3 -I - "$out/$1.txt" "$want" <<'PY'
import re, sys
raw = open(sys.argv[1], encoding="utf-8", errors="replace").read()
titles = [t.strip().strip('"') for line in raw.splitlines() for t in line.split(", ")]
titles = [t for t in titles if t and t != "kwant"]
local = [t for t in titles if "This Mac" in t or "This computer" in t]
server = [t for t in titles if re.search(re.escape(sys.argv[2]) + r"(?![0-9])", t)]
print("titles:", titles)
missing = (["This Mac/This computer"] if not local else []) + ([sys.argv[2]] if not server else [])
if missing:
    print("missing:", ", ".join(missing))
    sys.exit(1)
PY
}

quit_app() {
  local pid=$1
  if [ $mac = 1 ]; then
    osascript -e 'tell application id "si.kwant.mac" to quit' || true
  else
    kill -TERM "$pid" 2> /dev/null || true
  fi
  for _ in $(seq 1 30); do kill -0 "$pid" 2> /dev/null || { wait "$pid" 2> /dev/null; return 0; }; sleep 1; done
  note "the app didn't quit within 30 s; stopping it"
  kill -9 "$pid" 2> /dev/null || true
  wait "$pid" 2> /dev/null || true
  return 1
}

# The proxy: listen port, kwant port, log file.
cat > "$work/proxy.py" <<'PY'
import re, socket, sys, threading
listen, target, log = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
lock = threading.Lock()
METHOD = re.compile(r"^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS) ")
def pump(src, dst, logging):
    try:
        while True:
            data = src.recv(65536)
            if not data:
                break
            if logging:
                lines = data.split(b"\r\n\r\n", 1)[0].decode("latin-1").split("\r\n")
                if METHOD.match(lines[0]):
                    ua = next((l.split(":", 1)[1].strip() for l in lines[1:] if l.lower().startswith("user-agent:")), "-")
                    with lock, open(log, "a") as f:
                        f.write(f"{lines[0]} | {ua}\n")
            dst.sendall(data)
    except OSError:
        pass
    finally:
        for s in (src, dst):
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
def handle(client):
    try:
        upstream = socket.create_connection(("127.0.0.1", target))
    except OSError:
        client.close()
        return
    threading.Thread(target=pump, args=(client, upstream, True), daemon=True).start()
    pump(upstream, client, False)
server = socket.socket()
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(("127.0.0.1", listen))
server.listen(64)
while True:
    conn, _ = server.accept()
    threading.Thread(target=handle, args=(conn,), daemon=True).start()
PY

kwant_port=$(free_port)
proxy_port=$(free_port)
origin="http://127.0.0.1:$proxy_port"
mkdir -p "$work/server"
KWNT_PORT=$kwant_port KWNT_DATA_DIR="$work/server" "$app" --serve-headless > "$out/server.out" 2>&1 &
server_pid=$!
python3 -I "$work/proxy.py" "$proxy_port" "$kwant_port" "$out/server-requests.log" > "$out/proxy.out" 2>&1 &
proxy_pid=$!
cleanup() { kill "$server_pid" "$proxy_pid" 2> /dev/null || true; rm -rf "$work"; }
trap cleanup EXIT
if ! wait_health "$proxy_port" 90; then
  fail "the second kwant (--serve-headless) never answered"; cat "$out/server.out"; exit 1
fi
note "the server answers at $origin: $(health "$proxy_port")"
: > "$out/server-requests.log"

# Waits until the server's log has a page request from a WebKit window (not the app's own check).
window_loaded() {
  for _ in $(seq 1 "$1"); do
    grep -E '^GET [^ ]+ HTTP/[0-9.]+ \| .*AppleWebKit' "$out/server-requests.log" > /dev/null 2>&1 && return 0
    sleep 1
  done
  return 1
}

# 1. The old desktop-target.json ("local, last server o") moves to the places file.
mig="$work/migrate"
mkdir -p "$mig"
printf '{"mode":"local","last_origin":"%s"}' "$origin" > "$mig/desktop-target.json"
KWNT_DATA_DIR="$mig" "$app" > "$out/migrate.out" 2>&1 &
pid=$!
for _ in $(seq 1 60); do [ -f "$mig/desktop-places.json" ] && break; sleep 1; done
wait_health 8799 90 || note "This Mac didn't answer during the move check"
cp "$mig/desktop-places.json" "$out/migrated-places.json" 2> /dev/null || true
if python3 -I - "$mig/desktop-places.json" "$origin" <<'PY'
import json, sys
p = json.load(open(sys.argv[1]))
want = {"this_mac": True, "servers": [sys.argv[2]], "open": ["this_mac"]}
got = {k: p.get(k) for k in want}
print("migrated:", got)
sys.exit(0 if got == want else 1)
PY
then note "desktop-target.json moved to servers=[$origin], open=[this_mac]"
else fail "desktop-target.json didn't move as expected"; fi
[ -e "$mig/desktop-target.json" ] && fail "the old desktop-target.json is still there"
quit_app "$pid" || true
cp "$mig/$applog" "$out/migrate-$applog" 2> /dev/null || true

# 2. This Mac and the server side by side, from the places file.
data="$work/places"
mkdir -p "$data"
printf '{"version":1,"this_mac":true,"servers":["%s"],"open":["this_mac","%s"]}' "$origin" "$origin" > "$data/desktop-places.json"
run() {
  local n=$1
  KWNT_DATA_DIR="$data" "$app" > "$out/app-$n.out" 2>&1 &
  pid=$!
  local started=$SECONDS
  if wait_health 8799 120; then note "run $n: This Mac answers: $(health 8799)"; else fail "run $n: This Mac's kwant never answered /api/health"; fi
  if window_loaded 90; then
    note "run $n: the server's window loaded its pages:"
    grep -E 'AppleWebKit' "$out/server-requests.log" | head -5
  else
    fail "run $n: the server never saw a page request from the app's window"
  fi
  local left=$(( 20 - (SECONDS - started) ))
  [ $left -gt 0 ] && sleep $left
  sleep 3
  kill -0 "$pid" 2> /dev/null || fail "run $n: the app exited within 20 s"
  windows "windows-$n"
  windows_check "windows-$n" || fail "run $n: the windows aren't This Mac/This computer and ${origin#http://}: $(tr '\n' ' ' < "$out/windows-$n.txt")"
  sleep 2
  shot "side-by-side-$n"
  quit_app "$pid" || note "run $n: quit by Apple event/SIGTERM didn't end it"
  cp "$data/desktop-places.json" "$out/places-after-$n.json" 2> /dev/null || true
  mv "$out/server-requests.log" "$out/server-requests-$n.log"
  : > "$out/server-requests.log"
}
run 1
python3 -I - "$data/desktop-places.json" "$origin" <<'PY' || fail "after quit, open doesn't list both places"
import json, sys
p = json.load(open(sys.argv[1]))
print("after quit:", p)
sys.exit(0 if p.get("open") == ["this_mac", sys.argv[2]] and p.get("servers") == [sys.argv[2]] else 1)
PY
# 3. Reopen: both come back with nothing typed.
run 2
cp "$data/$applog" "$out/$applog" 2> /dev/null || true
ls -l "$out"
exit $failed
