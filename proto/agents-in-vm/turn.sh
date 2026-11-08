#!/usr/bin/env bash
# One claude turn inside kwnt-proto-lin-agent, with the flags src/drivers/cli.rs passes.
#   turn.sh api|oauth "<prompt>" [resume-session-id]
# The container only ever gets a dummy credential; the broker swaps in the real one.
set -euo pipefail
MODE=$1; PROMPT=$2; RESUME=${3:-}
case $MODE in
  api)   CRED=(-e ANTHROPIC_API_KEY=sk-ant-api03-kwnt-dummy) ;;
  oauth) CRED=(-e CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-kwnt-dummy) ;;
  *) echo "mode api|oauth" >&2; exit 2 ;;
esac
EXTRA=(); [ -n "$RESUME" ] && EXTRA=(--resume "$RESUME")
docker exec -i -w /workspace \
  -e HOME=/home/agent -e CLAUDE_CONFIG_DIR=/home/agent/.claude \
  -e ANTHROPIC_BASE_URL=http://127.0.0.1:8080/anthropic "${CRED[@]}" \
  -e CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 -e DISABLE_AUTOUPDATER=1 \
  -e CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR=1 -e MCP_TOOL_TIMEOUT=330000 -e MCP_TIMEOUT=30000 \
  kwnt-proto-lin-agent \
  claude --model claude-sonnet-4-5 --effort medium --permission-mode default --strict-mcp-config \
    --disallowedTools CronCreate,CronDelete,CronList,Task \
    --mcp-config /run/kwnt/mcp.json --permission-prompt-tool mcp__kwnt__approve \
    --append-system-prompt "You are a kwant bot (spike)." \
    --output-format stream-json --verbose --include-partial-messages \
    "${EXTRA[@]}" -p "$PROMPT" < /dev/null
