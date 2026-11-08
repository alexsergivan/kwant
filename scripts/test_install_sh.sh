#!/bin/sh
# Runs site/install.sh in clean containers against a local mock release server that serves a real
# kwnt build, and checks: a bad checksum installs nothing, a good install starts kwnt and prints the
# sign-in link and Mira's lines, a second run upgrades in place, the systemd user-service path, and
# that sudo is never called. Nothing is downloaded from a real release; no host port is opened.
#
#   scripts/test_install_sh.sh path/to/kwnt [image ...]     (default images: ubuntu:22.04 debian:12)
#
# The kwnt binary must match the container's architecture (the host's, without emulation).
set -eu

[ $# -ge 1 ] || { echo "usage: $0 path/to/kwnt [image ...]" >&2; exit 2; }
bin=$1
shift
[ -x "$bin" ] || { echo "not an executable: $bin" >&2; exit 2; }
[ $# -gt 0 ] || set -- ubuntu:22.04 debian:12

root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
names=""
cleanup() {
  for n in $names; do docker rm -f "$n" >/dev/null 2>&1 || true; done
  rm -rf "$work"
}
trap cleanup EXIT

# The mock release: good/ (v1), v2/ (the same program with bytes appended, so its checksum differs
# and it still runs) and bad/ (v1 with a wrong .sha256)
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) echo "no prebuilt kwnt for $(uname -m)" >&2; exit 2 ;;
esac
tarball="kwnt-linux-$arch.tar.gz"
pack() { # pack DIR BINARY
  mkdir -p "$work/release/$1" "$work/stage-$1"
  cp "$2" "$work/stage-$1/kwnt"
  chmod 755 "$work/stage-$1/kwnt"
  tar -czf "$work/release/$1/$tarball" -C "$work/stage-$1" kwnt
  (cd "$work/release/$1" && sha256sum "$tarball" >"$tarball.sha256")
}
pack good "$bin"
cp "$bin" "$work/kwnt-v2"
printf 'kwant install test v2' >>"$work/kwnt-v2"
pack v2 "$work/kwnt-v2"
pack bad "$bin"
printf '%064d  %s\n' 0 "$tarball" >"$work/release/bad/$tarball.sha256"
cp "$root/site/install.sh" "$work/install.sh"
cp "$root/scripts/test_install_sh.inner.sh" "$work/inner.sh"

fails=0
for image in "$@"; do
  name="kwnt-install-test-$$-$(printf '%s' "$image" | tr -c 'a-z0-9' '-')"
  names="$names $name"
  echo "=== $image"
  if docker run --rm --name "$name" --network bridge -v "$work:/t:ro" "$image" sh /t/inner.sh; then
    echo "=== $image: PASS"
  else
    echo "=== $image: FAIL"
    fails=$((fails + 1))
  fi
done
[ "$fails" -eq 0 ]
