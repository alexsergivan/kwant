#!/usr/bin/env bash
# The tests that need Docker and kwant's own images: the agents-in-VM security tests, the VM
# browser container and the parallel harness. CI's docker-tests job builds the images and runs
# them strict (CI=true); the rust job skips them by name, because it has no images.
#
#   scripts/ci_docker_tests.sh images      build the executor, agent and browser images, pull oha
#   scripts/ci_docker_tests.sh run         run the Docker tests (strict: a missing image fails)
#   scripts/ci_docker_tests.sh rust-tests  the rust job's `cargo test`, without the Docker tests
#   scripts/ci_docker_tests.sh tools       install what the strict rust job's tests need (apt)
#   scripts/ci_docker_tests.sh both-ways   a browser test with Chrome hidden: fails with CI=true, skips without
#
# Run it from the repo root. Locally, `images` then `KWNT_TEST_REQUIRE=1 scripts/ci_docker_tests.sh run`.
set -euo pipefail

AGENT_VM_UNIT_TEST=sandbox::agent_vm::tests::in_a_real_container_a_hook_cant_be_written_but_git_commit_works
# Tests in the default run that skip (or fail, in CI) without Docker and the agent image.
DOCKER_TESTS=(
  the_three_attacks_fail_in_a_shell_and_in_the_agents_own_bash
  nothing_the_bot_started_outlives_the_containers_last_turn
  an_approval_goes_through_kwnt_and_both_answers_work
  every_failure_is_a_notice_and_nothing_runs_on_this_computer
  "$AGENT_VM_UNIT_TEST"
)

# Image tags come from the code, so a bump there needs no edit here.
const() { sed -n "s/.*pub const $1: &str = \"\\([^\"]*\\)\".*/\\1/p" "$2" | head -1; }
CLAUDE_VERSION=$(const CLAUDE_VERSION src/sandbox/agent_vm/mod.rs)
AGENT_IMAGE="kwnt-agent:v1-claude-$CLAUDE_VERSION"
BROWSER_IMAGE=$(const BROWSER_IMAGE src/sandbox/service.rs)
EXECUTOR_IMAGE=$(const EXECUTOR_IMAGE src/sandbox/manager.rs)
OHA_IMAGE=$(const OHA_IMAGE src/parallel/harness.rs)
for v in CLAUDE_VERSION BROWSER_IMAGE EXECUTOR_IMAGE OHA_IMAGE; do
  [ -n "${!v}" ] || { echo "ci_docker_tests.sh: could not read $v from the source" >&2; exit 1; }
done

# Runs a step and records how long it took, in the job summary too when there is one.
timed() {
  local name=$1 start end rc=0; shift
  start=$(date +%s)
  "$@" || rc=$?
  end=$(date +%s)
  echo "$name: $((end - start)) s (exit $rc)"
  [ -z "${GITHUB_STEP_SUMMARY:-}" ] || echo "| $name | $((end - start)) s | exit $rc |" >> "$GITHUB_STEP_SUMMARY"
  return "$rc"
}

images() {
  [ -z "${GITHUB_STEP_SUMMARY:-}" ] || printf '| step | time | result |\n|---|---|---|\n' >> "$GITHUB_STEP_SUMMARY"
  local all
  all=$(date +%s)
  timed "$EXECUTOR_IMAGE" docker build --progress plain -t "$EXECUTOR_IMAGE" -f docker/executor.Dockerfile docker
  timed "$AGENT_IMAGE" docker build --progress plain -t "$AGENT_IMAGE" --build-arg "CLAUDE_VERSION=$CLAUDE_VERSION" \
    -f docker/agent.Dockerfile docker
  timed "$BROWSER_IMAGE" docker build --progress plain -t "$BROWSER_IMAGE" -f docker/browser.Dockerfile docker
  timed "oha (pull)" docker pull --quiet "$OHA_IMAGE"
  echo "all images: $(( $(date +%s) - all )) s"
  [ -z "${GITHUB_STEP_SUMMARY:-}" ] || echo "| all images | $(( $(date +%s) - all )) s | |" >> "$GITHUB_STEP_SUMMARY"
}

run() {
  export KWNT_TEST_REQUIRE=1
  [ -z "${GITHUB_STEP_SUMMARY:-}" ] || printf '| tests | time | result |\n|---|---|---|\n' >> "$GITHUB_STEP_SUMMARY"
  local failed=()
  # Every group runs even when an earlier one fails; the script fails at the end
  each() { local name=$1; shift; timed "$name" "$@" || failed+=("$name"); }
  each "agents-in-VM tests" cargo test --locked --no-fail-fast --test agents_in_vm_adversarial_test \
    --test agents_in_vm_fail_closed_test
  each "agent_vm unit test" cargo test --locked --lib -- --exact "$AGENT_VM_UNIT_TEST"
  # Ignored in the default run because they start containers; here the images exist
  each "browser container tests" cargo test --locked --no-fail-fast --test browser_container_test \
    --test browser_container_download_test -- --ignored
  each "browser stream (live Chromium)" cargo test --locked --test browser_stream_test -- --ignored --exact \
    live_chromium_stream_and_isolated_sessions
  each "parallel harness" cargo test --locked --test parallel_test -- --ignored
  if [ ${#failed[@]} -gt 0 ]; then
    printf 'ci_docker_tests.sh: failed: %s\n' "${failed[@]}" >&2
    exit 1
  fi
}

# Chrome, git and python3 are on GitHub's Ubuntu image; ffmpeg and Xvfb may not be. Without them
# the video and window tests fail in a strict run, so install what's missing.
tools() {
  local need=()
  command -v ffmpeg >/dev/null || need+=(ffmpeg)
  command -v Xvfb >/dev/null || need+=(xvfb)
  if [ ${#need[@]} -gt 0 ]; then
    timed "apt install ${need[*]}" sudo sh -c "apt-get update -q && apt-get install -y -q --no-install-recommends ${need[*]}"
  fi
  for t in google-chrome git python3 ffmpeg Xvfb; do
    printf '%s: %s\n' "$t" "$(command -v "$t" || echo MISSING)"
  done
}

# Both branches of skip_or_fail on a real browser test, with Chrome hidden (empty PATH, empty HOME,
# no KWNT_CHROME_PATH): with CI=true it must fail and name the fix; without CI it must pass, saying
# it skipped. Runs the binary `rust-tests` already built.
PROOF_TEST=browser_keys_test
PROOF_NAME=a_bots_enter_submits_the_form_and_other_keys_still_type
both_ways() {
  local bin home out rc
  bin=$(cargo test --locked --no-run --message-format=json --test "$PROOF_TEST" 2>/dev/null | python3 -c '
import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["name"] == sys.argv[1]:
        print(m["executable"])' "$PROOF_TEST")
  [ -x "$bin" ] || { echo "both-ways: no $PROOF_TEST binary" >&2; exit 1; }
  home=$(mktemp -d)
  echo "== CI=true, no Chrome: must FAIL"
  rc=0; out=$(env -i HOME="$home" PATH=/nonexistent CI=true "$bin" --exact "$PROOF_NAME" 2>&1) || rc=$?
  echo "$out"; echo "exit $rc"
  [ "$rc" -ne 0 ] || { echo "both-ways: the strict run passed without Chrome" >&2; exit 1; }
  grep -q "Chrome or Chromium is missing, and this run is strict" <<<"$out" \
    || { echo "both-ways: the strict run failed for another reason" >&2; exit 1; }
  echo "== no CI, no Chrome: must SKIP (pass, saying so)"
  rc=0; out=$(env -i HOME="$home" PATH=/nonexistent "$bin" --exact "$PROOF_NAME" --nocapture 2>&1) || rc=$?
  echo "$out"; echo "exit $rc"
  [ "$rc" -eq 0 ] && grep -q "skipped: Chrome or Chromium is missing" <<<"$out" \
    || { echo "both-ways: the local run didn't skip" >&2; exit 1; }
  rm -rf "$home"
  echo "both-ways: strict fails, local skips"
}

rust_tests() {
  local skip=()
  for t in "${DOCKER_TESTS[@]}"; do skip+=(--skip "$t"); done
  cargo test --locked --no-fail-fast -- "${skip[@]}"
}

case "${1:-}" in
  images) images ;;
  run) run ;;
  rust-tests) rust_tests ;;
  tools) tools ;;
  both-ways) both_ways ;;
  list) printf '%s\n' "${DOCKER_TESTS[@]}" ;;
  *) sed -n '2,12p' "$0" >&2; exit 2 ;;
esac
