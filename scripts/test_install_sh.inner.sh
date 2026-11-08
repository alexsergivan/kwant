#!/bin/sh
# Runs inside a clean container for scripts/test_install_sh.sh, as root. /t (read only) holds
# install.sh and the mock release folders good/, v2/ and bad/.
set -eu

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends curl ca-certificates python3 procps >/dev/null

# The mock release server, on the container's own loopback
(cd /t/release && exec python3 -m http.server 18080 --bind 127.0.0.1) >/tmp/http.log 2>&1 &
i=0
until curl -fsS -o /dev/null http://127.0.0.1:18080/good/ 2>/dev/null; do
  i=$((i + 1))
  [ "$i" -lt 50 ] || { echo "mock release server didn't start"; cat /tmp/http.log; exit 1; }
  sleep 0.2
done

# A sudo that only records being called: install.sh must never call it
cat >/usr/local/bin/sudo <<'EOF'
#!/bin/sh
echo "sudo $*" >>/tmp/sudo-called
exit 1
EOF
chmod 755 /usr/local/bin/sudo

useradd -m -s /bin/sh tester
home=/home/tester

pass=0
check() { # check DESCRIPTION COMMAND...
  d=$1
  shift
  if "$@"; then
    echo "  ok   $d"
    pass=$((pass + 1))
  else
    echo "  FAIL $d"
    echo "--- output"
    cat /tmp/out 2>/dev/null || true
    echo "--- kwnt.log"
    tail -n 30 "$home/.kwnt/kwnt.log" 2>/dev/null || true
    exit 1
  fi
}

# install RELEASE_DIR [EXTRA_PATH]: `cat install.sh | sh` as tester, the way `curl … | sh` runs it
install() {
  extra=${2:-}
  su tester -c "cd && cat /t/install.sh | env PATH='$extra/usr/local/bin:/usr/bin:/bin' \
    KWNT_INSTALL_BASE_URL=http://127.0.0.1:18080/$1 KWNT_INSTALL_SETUP_FLAGS=--dry-run \
    KWNT_LAYA_MANAGED=0 KWNT_PORT=18799 sh" >/tmp/out 2>&1
}

sha() { sha256sum "$1" | awk '{print $1}'; }
health() { curl -fsS --max-time 3 http://127.0.0.1:18799/api/health 2>/dev/null | grep -q '"app":"kwnt"'; }
v1=$(cd /tmp && tar -xzf /t/release/good/kwnt-linux-*.tar.gz kwnt -O | sha256sum | awk '{print $1}')
v2=$(cd /tmp && tar -xzf /t/release/v2/kwnt-linux-*.tar.gz kwnt -O | sha256sum | awk '{print $1}')

echo "- a bad checksum installs nothing"
rc=0
install bad || rc=$?
check "the install fails" [ "$rc" -ne 0 ]
check "it says nothing was installed" grep -q "didn't match its checksum, so nothing was installed" /tmp/out
check "no kwnt in ~/.local/bin" [ ! -e "$home/.local/bin/kwnt" ]
check "nothing is listening" sh -c '! curl -fsS --max-time 2 http://127.0.0.1:18799/api/health >/dev/null 2>&1'

echo "- a good install (no systemd: runs in the background)"
install good
check "kwnt is in ~/.local/bin" [ -x "$home/.local/bin/kwnt" ]
check "it is the released binary" [ "$(sha "$home/.local/bin/kwnt")" = "$v1" ]
check "/api/health says app kwnt" health
check "first line names kwant and the arch" grep -Eq '^✓ kwant [0-9][^ ]* for Linux (x86_64|arm64)$' /tmp/out
check "says it runs in the background" grep -q '^✓ Runs in the background' /tmp/out
check "Mira's background line" grep -qx '✓ Laya and a browser for the bots are getting ready in the background' /tmp/out
link_line='^Open http://127\.0\.0\.1:18799/#kwnt_token=[A-Za-z0-9._-]{16,} in a browser on this machine\.$'
check "the one-time link, to open on this machine" grep -Eq "$link_line" /tmp/out
check "on a server: the ssh tunnel first" grep -qx 'On a server? On your laptop, first run:' /tmp/out
check "the tunnel uses the port and this user" grep -qx '  ssh -N -L 18799:127.0.0.1:18799 tester@<this-server>' /tmp/out
check "then the same link; HTTPS for a phone" grep -qx 'then open the same link there. For a phone (HTTPS), see https://kwant.si/docs/install' /tmp/out
check "how long the link lasts" grep -qx 'The link works once, for 15 minutes.' /tmp/out
check "no 'phone or laptop' for a loopback link" sh -c '! grep -q "Open this on your phone or laptop" /tmp/out'
check "no QR code for a loopback link" sh -c '! grep -q "█" /tmp/out'
check "not running outside the installer" sh -c '! grep -q "already running outside this installer" /tmp/out'
check "Mira's closing lines" grep -qx 'There you choose your AI and sign in. Nothing else to type here.' /tmp/out
check "how to get another link" grep -Eq '^Lost the link\? Run: .*kwnt signin-link$' /tmp/out
check "no kwnt serve banner in the output" sh -c '! grep -q "To sign in from another device" /tmp/out'
pid1=$(cat "$home/.kwnt/kwnt.pid")
check "the pid file names a running kwnt" grep -q kwnt "/proc/$pid1/cmdline"
i=0
while [ ! -s "$home/.kwnt/setup.log" ] && [ "$i" -lt 30 ]; do sleep 1; i=$((i + 1)); done
check "kwnt setup ran in the background" [ -s "$home/.kwnt/setup.log" ]

echo "- running it again upgrades in place"
install v2
check "the binary is the new one" [ "$(sha "$home/.local/bin/kwnt")" = "$v2" ]
check "/api/health answers" health
pid2=$(cat "$home/.kwnt/kwnt.pid")
check "kwnt was restarted" [ "$pid2" != "$pid1" ]
check "the old process is gone" sh -c "! kill -0 $pid1 2>/dev/null"
check "a new link was printed" grep -Eq "$link_line" /tmp/out
check "only one kwnt serve runs" [ "$(pgrep -f 'kwnt serve' | wc -l)" -eq 1 ]
check "not running outside the installer" sh -c '! grep -q "already running outside this installer" /tmp/out'
kill "$pid2"
i=0
while kill -0 "$pid2" 2>/dev/null && [ "$i" -lt 20 ]; do sleep 0.5; i=$((i + 1)); done

# stop_all: every kwnt this user runs, by the installer or by hand
stop_all() {
  pkill -u tester -f 'kwnt(-hand)? serve' 2>/dev/null || true
  i=0
  while pgrep -u tester -f 'kwnt(-hand)? serve' >/dev/null && [ "$i" -lt 20 ]; do sleep 0.5; i=$((i + 1)); done
}
outside_line='kwnt is already running outside this installer. Restart it to use the new version.'

echo "- a kwnt started by hand (no pid file): found by its process"
rm -f "$home/.kwnt/kwnt.pid"
su tester -c "cd && KWNT_DATA_DIR=$home/hand KWNT_PORT=18800 KWNT_LAYA_MANAGED=0 nohup $home/.local/bin/kwnt serve >/tmp/hand.log 2>&1 </dev/null &"
i=0
until curl -fsS --max-time 2 http://127.0.0.1:18800/api/health 2>/dev/null | grep -q '"app":"kwnt"'; do
  i=$((i + 1))
  [ "$i" -lt 60 ] || { cat /tmp/hand.log; exit 1; }
  sleep 0.5
done
install good
check "says it runs outside the installer" grep -qx "$outside_line" /tmp/out
check "doesn't say updated" sh -c '! grep -q "(updated from" /tmp/out'
check "the hand-started kwnt still runs" curl -fsS --max-time 2 -o /dev/null http://127.0.0.1:18800/api/health
check "the link is still printed" grep -Eq "$link_line" /tmp/out
stop_all

echo "- a kwnt started by hand under another name: found by its port"
rm -f "$home/.kwnt/kwnt.pid"
su tester -c "cp $home/.local/bin/kwnt $home/kwnt-hand && cd && KWNT_PORT=18799 KWNT_LAYA_MANAGED=0 nohup $home/kwnt-hand serve >/tmp/hand.log 2>&1 </dev/null &"
i=0
until health; do
  i=$((i + 1))
  [ "$i" -lt 60 ] || { cat /tmp/hand.log; exit 1; }
  sleep 0.5
done
install v2
check "says it runs outside the installer" grep -qx "$outside_line" /tmp/out
check "doesn't say updated" sh -c '! grep -q "(updated from" /tmp/out'
check "the hand-started kwnt still answers" health
stop_all

echo "- with a systemd user session: a service, and the linger command when it needs admin"
mkdir -p /tmp/stubs
# systemctl and loginctl stand-ins: the user manager "runs" ExecStart in the background
cat >/tmp/stubs/systemctl <<'EOF'
#!/bin/sh
echo "systemctl $*" >>"$HOME/systemctl.calls"
[ "$1" = --user ] || exit 1
case "$2" in
  show-environment | daemon-reload | enable) exit 0 ;;
  restart)
    unit="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/$3"
    [ -f "$unit" ] || exit 1
    if [ -f "$HOME/unit.pid" ]; then kill "$(cat "$HOME/unit.pid")" 2>/dev/null; sleep 1; fi
    exe=$(sed -n 's/^ExecStart=//p' "$unit" | sed "s#%h#$HOME#g")
    # shellcheck disable=SC2086
    nohup $exe >>"$HOME/unit.log" 2>&1 </dev/null &
    echo $! >"$HOME/unit.pid"
    ;;
  *) exit 1 ;;
esac
EOF
cat >/tmp/stubs/loginctl <<'EOF'
#!/bin/sh
echo "loginctl $*" >>"$HOME/loginctl.calls"
case "$1" in
  show-user) echo "Linger=no" ;;
  enable-linger) echo "Access denied" >&2; exit 1 ;;
esac
EOF
chmod 755 /tmp/stubs/systemctl /tmp/stubs/loginctl
rm -f "$home/.kwnt/kwnt.pid"
install good /tmp/stubs:
unit="$home/.config/systemd/user/kwnt.service"
check "the unit file is written" [ -f "$unit" ]
check "it runs ~/.local/bin/kwnt serve" grep -qx 'ExecStart=%h/.local/bin/kwnt serve' "$unit"
check "it reads ~/.kwnt/kwnt.env if present" grep -qx 'EnvironmentFile=-%h/.kwnt/kwnt.env' "$unit"
check "it is enabled and restarted" sh -c "grep -q 'enable kwnt.service' $home/systemctl.calls && grep -q 'restart kwnt.service' $home/systemctl.calls"
check "linger was tried without a password prompt" grep -q 'enable-linger --no-ask-password tester' "$home/loginctl.calls"
check "says it runs as a service" grep -qx '✓ Runs as a service for this user' /tmp/out
check "prints the one admin command" grep -q 'runs once: sudo loginctl enable-linger tester$' /tmp/out
check "/api/health answers" health
check "the downgrade back to v1 replaced the binary" [ "$(sha "$home/.local/bin/kwnt")" = "$v1" ]
check "the link is printed" grep -Eq "$link_line" /tmp/out
kill "$(cat "$home/unit.pid")" 2>/dev/null || true

echo "- never sudo"
check "sudo was never called" [ ! -e /tmp/sudo-called ]
# Every line that says sudo is a comment or text it prints, never a command it runs
check "install.sh runs no sudo" sh -c "! grep -n sudo /t/install.sh | grep -Ev '^[0-9]+: *#|linger_cmd=\"sudo |# Never uses sudo'"

echo "PASS: $pass checks"
