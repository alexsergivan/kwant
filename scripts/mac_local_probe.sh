#!/bin/bash
# 0.8.0 079-mac-local-hardening, run by .github/workflows/mac-local.yml on GitHub's macOS runner (the
# only Mac CI has). It takes the exact settings kwnt gives Claude on this computer on a Mac (written
# by the ignored test drivers::mac_local::tests::write_settings_for_the_mac_ci_probe, Seatbelt on)
# and checks three things:
#
#   1. The real Claude Code (CLAUDE_VERSION) accepts them: its --debug log says it added the deny
#      rules. No Claude sign-in exists here, so no model turn runs.
#   2. Seatbelt enforces the sandbox half: the same filesystem/network sections are handed to
#      sandbox-runtime (srt, the library Claude's own Bash sandbox is built on), and reads/writes of
#      the denied places must fail while an ordinary file in the home folder still works.
#   3. The Keychain: whether a sandboxed `security find-generic-password` still gets a secret. This
#      is reported, not enforced: denyRead of ~/Library/Keychains only stops reading the files.
#
# Usage: scripts/mac_local_probe.sh <kwnt lib test binary>
# Needs: node >= 22.12 and npm on PATH, macOS. Writes a summary to $GITHUB_STEP_SUMMARY when set.
set -uo pipefail

BIN="$1"
CLAUDE_VERSION="${CLAUDE_VERSION:-2.1.316}"
SRT_VERSION="${SRT_VERSION:-0.0.81}"
T="${RUNNER_TEMP:-$(mktemp -d)}/mac-local-probe"
REAL_HOME="$HOME"
H="$T/home"
D="$T/data"
SUMMARY="${GITHUB_STEP_SUMMARY:-/dev/null}"
fails=0

rm -rf "$T" && mkdir -p "$T"
note() { echo "$*"; echo "$*" >> "$SUMMARY"; }
pass() { note "- PASS: $*"; }
fail() { note "- FAIL: $*"; echo "::error::$*"; fails=$((fails + 1)); }

note "## Mac local hardening probe (macOS $(sw_vers -productVersion), $(uname -m))"

# A stand-in home and data folder with something to steal in each denied place
mkdir -p "$H/.claude" "$H/.codex" "$H/.grok" "$H/Library/Keychains" "$H/work" "$D/profiles/p1"
echo SECRET-claude > "$H/.claude/kwnt-probe-secret.txt"
echo SECRET-codex > "$H/.codex/auth.json"
echo SECRET-grok > "$H/.grok/user-settings.json"
echo SECRET-keychain-file > "$H/Library/Keychains/probe.keychain-db"
echo SECRET-token > "$D/api-token"
echo SECRET-db > "$D/kwnt.db"
echo SECRET-profile > "$D/profiles/p1/.credentials.json"
echo ok > "$H/work/ok.txt"

settings_for() { # <home> <out>
    env HOME="$1" KWNT_DATA_DIR="$D" KWNT_MAC_SETTINGS_OUT="$2" \
        "$BIN" --ignored --exact drivers::mac_local::tests::write_settings_for_the_mac_ci_probe -q > "$2.log" 2>&1 \
        || { cat "$2.log"; return 1; }
}
settings_for "$H" "$T/settings.json" || { fail "couldn't write kwnt's settings"; exit 1; }
note ""
note "### kwnt's settings for a Mac turn (Seatbelt on)"
python3 -c 'import json,sys; s=json.load(open(sys.argv[1])); print(len(s["permissions"]["deny"]), "deny rules;", "sandbox.denyRead:", s["sandbox"]["filesystem"]["denyRead"])' "$T/settings.json" | tee -a "$SUMMARY"

# --- 1. The real Claude Code reads them ------------------------------------------------------
note ""
note "### 1. Claude Code $CLAUDE_VERSION accepts them"
npm install --silent --no-audit --no-fund --prefix "$T/cc" "@anthropic-ai/claude-code@$CLAUDE_VERSION" > "$T/cc.log" 2>&1 || { cat "$T/cc.log"; fail "npm install claude-code"; }
CLAUDE="$T/cc/node_modules/.bin/claude"
if [ -x "$CLAUDE" ]; then
    # No sign-in: Claude loads its settings, then fails to reach the model. 90 s at most.
    env -i HOME="$H" PATH="$(dirname "$(command -v node)"):/usr/bin:/bin" \
        perl -e 'alarm 90; exec @ARGV' "$CLAUDE" --debug --settings "$(cat "$T/settings.json")" -p hi < /dev/null > "$T/claude.out" 2>&1
    echo "claude exit $? (expected to fail: not signed in)"
    LOG=$(ls -t "$H/.claude/debug/"* 2>/dev/null | head -1)
    want=$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["permissions"]["deny"]))' "$T/settings.json")
    if [ -n "$LOG" ] && grep -q "Adding $want deny rule(s) to destination 'flagSettings'" "$LOG"; then
        pass "Claude's debug log: \"Adding $want deny rule(s) to destination 'flagSettings'\""
    else
        fail "Claude didn't report adding $want deny rules from --settings"
        [ -n "$LOG" ] && grep -i "settings\|deny\|invalid" "$LOG" | head -20
        head -40 "$T/claude.out"
    fi
    if [ -n "$LOG" ]; then
        note "Claude's sandbox lines (informational):"
        { grep -i "sandbox" "$LOG" | head -15 | sed 's/^/    /'; } | tee -a "$SUMMARY"
    fi
else
    fail "claude not installed"
fi

# --- 2. Seatbelt enforces the sandbox half (sandbox-runtime) ---------------------------------
note ""
note "### 2. Seatbelt (sandbox-runtime $SRT_VERSION) enforces the same sections"
npm install --silent --no-audit --no-fund --prefix "$T/srt" "@anthropic-ai/sandbox-runtime@$SRT_VERSION" > "$T/srt.log" 2>&1 || { cat "$T/srt.log"; fail "npm install sandbox-runtime"; }
SRT="$T/srt/node_modules/.bin/srt"
srt_settings() { # <kwnt settings> <out>: Claude's sandbox.filesystem/network use srt's own format
    python3 -c 'import json,sys; s=json.load(open(sys.argv[1]))["sandbox"]; json.dump({"filesystem": s["filesystem"], "network": dict(s["network"], deniedDomains=[])}, open(sys.argv[2], "w"), indent=1)' "$1" "$2"
}
in_sandbox() { # <srt settings> <home> <command...>: runs it, output in $T/out
    local f="$1" home="$2"; shift 2
    (cd "$home" && env HOME="$home" perl -e 'alarm 60; exec @ARGV' "$SRT" --settings "$f" "$@") > "$T/out" 2>&1
}
if [ -x "$SRT" ]; then
    srt_settings "$T/settings.json" "$T/srt.json"
    in_sandbox "$T/srt.json" "$H" cat "$H/work/ok.txt"
    if [ $? -eq 0 ] && grep -qx ok "$T/out"; then pass "an ordinary file in the home folder is still readable"; else fail "the sandbox didn't run an allowed read: $(head -5 "$T/out")"; fi
    for f in "$H/.claude/kwnt-probe-secret.txt" "$H/.codex/auth.json" "$H/.grok/user-settings.json" \
        "$H/Library/Keychains/probe.keychain-db" "$D/api-token" "$D/kwnt.db" "$D/profiles/p1/.credentials.json"; do
        in_sandbox "$T/srt.json" "$H" cat "$f"
        rc=$?
        if [ $rc -ne 0 ] && ! grep -q SECRET "$T/out"; then pass "read denied: ${f#$T/}"; else fail "read NOT denied: ${f#$T/} (exit $rc)"; fi
    done
    in_sandbox "$T/srt.json" "$H" sh -c "echo planted > '$H/.claude/settings.json'"
    if [ ! -e "$H/.claude/settings.json" ]; then pass "write denied: home/.claude/settings.json"; else fail "write NOT denied: home/.claude/settings.json"; fi
    in_sandbox "$T/srt.json" "$H" sh -c "echo new > '$H/work/new.txt'"
    if [ -e "$H/work/new.txt" ]; then pass "a write in the home folder still works"; else fail "a write in the home folder failed: $(head -5 "$T/out")"; fi
    in_sandbox "$T/srt.json" "$H" curl -sS -m 15 -o /dev/null https://example.com
    rc=$?
    if [ $rc -ne 0 ]; then pass "network to an unlisted domain blocked (curl exit $rc; in Claude this would ask)"; else fail "network to example.com NOT blocked"; fi
else
    fail "srt not installed"
fi

# --- 3. The Keychain (reported, not enforced) --------------------------------------------------
note ""
note "### 3. Keychain through the security service (reported only)"
KC_SERVICE="kwnt-ci-probe-$$"
if [ -x "$SRT" ] && security add-generic-password -a kwnt-ci -s "$KC_SERVICE" -w "probe-value-$$" -U > /dev/null 2>&1; then
    outside=$(security find-generic-password -s "$KC_SERVICE" -w 2>/dev/null)
    settings_for "$REAL_HOME" "$T/settings-real.json" && srt_settings "$T/settings-real.json" "$T/srt-real.json"
    login_kc=$(ls "$REAL_HOME"/Library/Keychains/login.keychain-db 2>/dev/null | head -1)
    if [ -n "$login_kc" ]; then
        in_sandbox "$T/srt-real.json" "$REAL_HOME" cat "$login_kc"
        if [ $? -ne 0 ]; then pass "reading the login keychain file is denied"; else fail "reading the login keychain file NOT denied"; fi
    fi
    in_sandbox "$T/srt-real.json" "$REAL_HOME" security find-generic-password -s "$KC_SERVICE" -w
    inside=$(cat "$T/out")
    if [ "$outside" != "probe-value-$$" ]; then
        note "- UNKNOWN: the probe item couldn't be read even outside the sandbox"
    elif printf '%s' "$inside" | grep -q "probe-value-$$"; then
        note "- NOT PROTECTED: a sandboxed \`security find-generic-password\` read a Keychain item (the Seatbelt profile allows the securityd service). Claude keeps its sign-in in the Keychain as \"Claude Code-credentials\"."
        echo "::warning::Keychain items are readable from inside the sandbox via the security service"
    else
        note "- BLOCKED: a sandboxed \`security find-generic-password\` got nothing: $(head -3 "$T/out")"
    fi
    security delete-generic-password -s "$KC_SERVICE" > /dev/null 2>&1 || true
else
    note "- UNKNOWN: couldn't add a probe item to this runner's Keychain"
fi

note ""
note "Result: $fails failure(s)"
exit $((fails > 0))
