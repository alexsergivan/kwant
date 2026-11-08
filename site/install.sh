#!/bin/sh
# kwant in one line, on your own server or Mac:
#
#   curl -fsSL https://kwant.si/install.sh | sh
#
# Linux (x86_64 or arm64, glibc 2.35+: Ubuntu 22.04+, Debian 12+): downloads the prebuilt kwnt from the
# latest release, checks it against its .sha256 (a mismatch installs nothing), puts it in ~/.local/bin,
# runs it as a systemd user service when this user has one (otherwise in the background, and says how to
# start it again), starts `kwnt setup --yes` in the background and prints a one-time sign-in link: a
# loopback one with the SSH tunnel to reach it from a laptop, any other with its QR code. A kwnt it
# didn't start (by hand, no pid file) keeps running; it says so instead of "updated".
# macOS: the same checked download of the Mac app, into ~/Applications.
# Never uses sudo. Running it again upgrades kwant in place and keeps your data (~/.kwnt).
#
# For tests: KWNT_INSTALL_BASE_URL is where the release files are (default: the latest GitHub release),
# and KWNT_INSTALL_SETUP_FLAGS is added to `kwnt setup --yes` (for example --dry-run).
#
# Everything is inside main(), called on the last line, so a download cut off half-way runs nothing.

# `a && b || fail …` is meant: fail when any step before it fails
# shellcheck disable=SC2015

set -u

say() { printf '%s\n' "$*"; }

fail() {
  printf 'kwant install: %s\n' "$*" >&2
  exit 1
}

# fetch URL FILE
fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 2 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    fail "This needs curl or wget to download kwant. Install one of them, then run this again."
  fi
}

# fetch_quiet URL: the body on stdout, or nothing
fetch_quiet() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsS --max-time 3 "$1" 2>/dev/null
  else
    wget -q -T 3 -O - "$1" 2>/dev/null
  fi
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    fail "This needs sha256sum or shasum to check the download. Nothing was installed."
  fi
}

# download_checked NAME DIR: NAME and NAME.sha256 from the release into DIR, or stop with nothing installed
download_checked() {
  fetch "$BASE/$1" "$2/$1" || fail "Couldn't download $BASE/$1. Nothing was installed."
  fetch "$BASE/$1.sha256" "$2/$1.sha256" || fail "Couldn't download $BASE/$1.sha256. Nothing was installed."
  want=$(awk '{print $1; exit}' "$2/$1.sha256")
  case "$want" in
    *[!0-9a-f]* | "") want=bad ;;
  esac
  got=$(sha256_of "$2/$1")
  if [ "${#want}" -ne 64 ] || [ "$got" != "$want" ]; then
    fail "The download didn't match its checksum, so nothing was installed. Try again in a minute; if it keeps happening, tell us at https://kwant.si/contact.html"
  fi
}

install_linux() {
  case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 label=x86_64 ;;
    aarch64 | arm64) arch=aarch64 label=arm64 ;;
    *) fail "The prebuilt kwant runs on x86_64 and arm64 Linux, not $(uname -m). Download kwant for your computer at https://kwant.si" ;;
  esac
  bindir="$HOME/.local/bin"
  data="$HOME/.kwnt"
  name="kwnt-linux-$arch.tar.gz"

  download_checked "$name" "$TMP"
  mkdir "$TMP/x" && tar -xzf "$TMP/$name" -C "$TMP/x" || fail "Couldn't unpack $name. Nothing was installed."
  [ -f "$TMP/x/kwnt" ] || fail "$name has no kwnt in it. Nothing was installed."
  chmod 755 "$TMP/x/kwnt"
  version=$("$TMP/x/kwnt" --version </dev/null 2>/dev/null) ||
    fail "This kwnt doesn't run here. It needs glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+). Nothing was installed."
  version=${version#kwnt }

  old=""
  if [ -x "$bindir/kwnt" ]; then
    old=$("$bindir/kwnt" --version </dev/null 2>/dev/null) || old=""
    old=${old#kwnt }
  fi
  mkdir -p "$bindir" "$data" || fail "Couldn't create $bindir or $data."
  # A new file renamed over the old one: a running kwnt keeps its own copy until it restarts
  cp "$TMP/x/kwnt" "$bindir/.kwnt.new" && chmod 755 "$bindir/.kwnt.new" && mv -f "$bindir/.kwnt.new" "$bindir/kwnt" ||
    fail "Couldn't write $bindir/kwnt."
  run_path="$bindir:$HOME/.grok/bin:/usr/local/bin:/usr/bin:/bin"

  # A kwnt this installer didn't start (no pidfile, no service) keeps running the old version
  port=$("$bindir/kwnt" status </dev/null 2>/dev/null | awk '$1 == "port" {print $2; exit}')
  port=${port:-8799}
  linger_cmd=""
  if command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then
    mode=service
    ours=$(systemctl --user show -p MainPID --value kwnt.service 2>/dev/null)
  else
    mode=background
    ours=$(cat "$data/kwnt.pid" 2>/dev/null)
  fi
  case "$ours" in "" | *[!0-9]*) ours=0 ;; esac
  outside=$(outside_kwnt "$ours")
  if [ "$mode" = service ]; then
    start_service
  else
    start_background
  fi

  # A browser, and Claude Code when no AI CLI is here yet: kwnt's own setup, never sudo (it prints
  # what needs admin rights). Laya isn't in the list because the service installs it on its first
  # start; two installs into the same folder at once would break each other.
  parts=browser
  if ! PATH="$run_path:$HOME/.claude/local" command -v claude >/dev/null 2>&1 &&
    ! PATH="$run_path" command -v codex >/dev/null 2>&1 &&
    ! PATH="$run_path" command -v grok >/dev/null 2>&1; then
    parts=browser,claude
  fi
  # shellcheck disable=SC2086 # the test flags are meant to split into words
  PATH="$run_path" nohup "$bindir/kwnt" setup --yes --only "$parts" ${KWNT_INSTALL_SETUP_FLAGS:-} >"$data/setup.log" 2>&1 </dev/null &

  tries=0
  until fetch_quiet "http://127.0.0.1:$port/api/health" | grep -q '"app":"kwnt"'; do
    tries=$((tries + 1))
    if [ "$tries" -ge 60 ]; then
      if [ "$mode" = service ]; then logs="journalctl --user -u kwnt"; else logs="tail $data/kwnt.log"; fi
      fail "kwant was installed but didn't answer on port $port within a minute. See what it says with: $logs. When it's up, run: $bindir/kwnt signin-link"
    fi
    sleep 1
  done

  run="kwnt"
  case ":$PATH:" in
    *":$bindir:"*) ;;
    *) run="$bindir/kwnt" ;;
  esac

  if [ -n "$outside" ]; then
    say "✓ kwant $version for Linux $label"
    say "kwnt is already running outside this installer. Restart it to use the new version."
  elif [ -n "$old" ] && [ "$old" != "$version" ]; then
    say "✓ kwant $version for Linux $label (updated from $old)"
  else
    say "✓ kwant $version for Linux $label"
  fi
  if [ "$mode" = service ] && [ -z "$linger_cmd" ]; then
    say "✓ Runs as a service for this user (starts after a reboot)"
  elif [ "$mode" = service ]; then
    say "✓ Runs as a service for this user"
    say "  To start it after a reboot too, someone with admin rights runs once: $linger_cmd"
  else
    say "✓ Runs in the background (this system has no user services)"
    say "  After a reboot, start it again with: nohup $bindir/kwnt serve >> $data/kwnt.log 2>&1 &"
  fi
  say "✓ Laya and a browser for the bots are getting ready in the background"
  say ""
  notice=$("$bindir/kwnt" signin-link --bare </dev/null) || fail "Couldn't make a sign-in link. Run: $run signin-link"
  link=$(printf '%s\n' "$notice" | awk '/:\/\// {print $1; exit}')
  case "$link" in
    http://127.0.0.1:* | http://localhost:* | "http://[::1]:"*)
      # Only this machine can open a loopback link (and its QR code): a laptop needs a tunnel
      say "Open $link in a browser on this machine."
      say "On a server? On your laptop, first run:"
      say "  ssh -N -L $port:127.0.0.1:$port $(id -un)@<this-server>"
      say "then open the same link there. For a phone (HTTPS), see https://kwant.si/docs/install"
      say ""
      say "The link works once, for 15 minutes."
      ;;
    *)
      say "Open this on your phone or laptop to finish. It works once, for 15 minutes:"
      say ""
      say "$notice"
      say ""
      ;;
  esac
  say "There you choose your AI and sign in. Nothing else to type here."
  say "Lost the link? Run: $run signin-link"
}

# outside_kwnt OURS: "yes" when this user runs a `kwnt serve` other than pid OURS, or, with no pid of
# ours, when something already answers as kwnt on the port
outside_kwnt() {
  me=$(id -u)
  for proc in /proc/[0-9]*; do
    [ "${proc#/proc/}" = "$1" ] && continue
    # The program and its first argument (cmdline is NUL-separated)
    args=$(tr '\0' '\n' <"$proc/cmdline" 2>/dev/null | head -n 2 | tr '\n' ' ') || continue
    prog=${args%% *}
    first=${args#* }
    first=${first%% *}
    if [ "${prog##*/}" = kwnt ] && [ "$first" = serve ] &&
      [ "$(awk '$1 == "Uid:" {print $2; exit}' "$proc/status" 2>/dev/null)" = "$me" ]; then
      echo yes
      return
    fi
  done
  if [ "$1" = 0 ] && fetch_quiet "http://127.0.0.1:$port/api/health" | grep -q '"app":"kwnt"'; then
    echo yes
  fi
}

start_service() {
  unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
  mkdir -p "$unit_dir" || fail "Couldn't create $unit_dir."
  cat >"$unit_dir/kwnt.service" <<UNIT || fail "Couldn't write $unit_dir/kwnt.service."
# Written by kwant's install.sh; running it again rewrites this file.
[Unit]
Description=kwant bots (kwnt serve)
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=%h/.local/bin/kwnt serve
Environment=PATH=$run_path
# Your own settings, one NAME=value per line (for example KWNT_ALLOWED_ORIGINS=https://kwant.example.com)
EnvironmentFile=-%h/.kwnt/kwnt.env
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT
  systemctl --user daemon-reload &&
    systemctl --user enable kwnt.service >/dev/null 2>&1 &&
    systemctl --user restart kwnt.service ||
    fail "Couldn't start the kwnt service. See: systemctl --user status kwnt"
  # Lingering keeps user services running after logout and starts them at boot
  user=$(id -un)
  if [ "$(loginctl show-user "$user" -p Linger 2>/dev/null)" != "Linger=yes" ] &&
    ! loginctl enable-linger --no-ask-password "$user" >/dev/null 2>&1; then
    linger_cmd="sudo loginctl enable-linger $user"
  fi
}

start_background() {
  pidfile="$data/kwnt.pid"
  if [ -f "$pidfile" ]; then
    pid=$(cat "$pidfile")
    case "$pid" in
      "" | *[!0-9]*) ;;
      *)
        if kill -0 "$pid" 2>/dev/null && grep -q kwnt "/proc/$pid/cmdline" 2>/dev/null; then
          kill "$pid" 2>/dev/null
          n=0
          while kill -0 "$pid" 2>/dev/null && [ "$n" -lt 10 ]; do
            sleep 1
            n=$((n + 1))
          done
        fi
        ;;
    esac
  fi
  PATH="$run_path" nohup "$bindir/kwnt" serve >>"$data/kwnt.log" 2>&1 </dev/null &
  echo $! >"$pidfile"
}

install_mac() {
  name="kwant_universal.app.tar.gz"
  download_checked "$name" "$TMP"
  mkdir "$TMP/x" && tar -xzf "$TMP/$name" -C "$TMP/x" || fail "Couldn't unpack $name. Nothing was installed."
  [ -d "$TMP/x/kwant.app" ] || fail "$name has no kwant.app in it. Nothing was installed."
  apps="$HOME/Applications"
  mkdir -p "$apps" || fail "Couldn't create $apps."
  rm -rf "$apps/.kwant.app.old"
  if [ -d "$apps/kwant.app" ]; then
    mv "$apps/kwant.app" "$apps/.kwant.app.old" || fail "Couldn't move the old kwant.app aside."
  fi
  if ! mv "$TMP/x/kwant.app" "$apps/kwant.app"; then
    [ -d "$apps/.kwant.app.old" ] && mv "$apps/.kwant.app.old" "$apps/kwant.app"
    fail "Couldn't put kwant.app in $apps."
  fi
  rm -rf "$apps/.kwant.app.old"
  say "✓ kwant for Mac in $apps/kwant.app"
  if [ -d /Applications/kwant.app ]; then
    say "  You also have kwant in /Applications. Keep one of them so you always open the new one."
  fi
  say ""
  say "Open kwant from $apps (or run: open \"$apps/kwant.app\"). If it's open now, quit it and open it again."
}

main() {
  BASE="${KWNT_INSTALL_BASE_URL:-https://github.com/alexsergivan/kwant/releases/latest/download}"
  BASE="${BASE%/}"
  [ -n "${HOME:-}" ] && [ -d "$HOME" ] || fail "HOME isn't set to a folder."
  TMP=$(mktemp -d 2>/dev/null || mktemp -d -t kwnt) || fail "Couldn't make a temporary folder."
  trap 'rm -rf "$TMP"' EXIT
  trap 'exit 1' INT TERM
  case "$(uname -s)" in
    Linux) install_linux ;;
    Darwin) install_mac ;;
    *) fail "This one-line install works on Linux and macOS, not on $(uname -s). Download kwant for your computer at https://kwant.si" ;;
  esac
}

main "$@"
